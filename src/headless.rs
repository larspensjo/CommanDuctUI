use crate::{
    AppEvent, BadgeDescriptor, ChartDataPacket, CheckState, ControlId, ControlStyle, DockStyle,
    LabelClass, LayoutRule, ListBoxItemDescriptor, ListBoxItemId, ListBoxRowDensity, MenuActionId,
    MessageSeverity, PlatformCommand, PlatformError, PlatformEventHandler, PlatformResult,
    SplitterOrientation, StyleId, TreeItemDescriptor, TreeItemId, UiStateProvider, WindowConfig,
    WindowId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type SharedHandler = Arc<Mutex<dyn PlatformEventHandler>>;
type SharedProvider = Arc<Mutex<dyn UiStateProvider>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogKind {
    SaveFile,
    OpenFile,
    ProfileSelection,
    Input,
    ExcludePatterns,
    Form,
    MessageBox,
    FolderPicker,
}

impl DialogKind {
    fn stable_name(&self) -> &'static str {
        match self {
            DialogKind::SaveFile => "save_file",
            DialogKind::OpenFile => "open_file",
            DialogKind::ProfileSelection => "profile_selection",
            DialogKind::Input => "input",
            DialogKind::ExcludePatterns => "exclude_patterns",
            DialogKind::Form => "form",
            DialogKind::MessageBox => "message_box",
            DialogKind::FolderPicker => "folder_picker",
        }
    }

    fn from_stable_name(name: &str) -> Option<Self> {
        match name {
            "save_file" => Some(DialogKind::SaveFile),
            "open_file" => Some(DialogKind::OpenFile),
            "profile_selection" => Some(DialogKind::ProfileSelection),
            "input" => Some(DialogKind::Input),
            "exclude_patterns" => Some(DialogKind::ExcludePatterns),
            "form" => Some(DialogKind::Form),
            "message_box" => Some(DialogKind::MessageBox),
            "folder_picker" => Some(DialogKind::FolderPicker),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogMatcher {
    pub kind: DialogKind,
    pub window_id: Option<WindowId>,
    pub title: Option<String>,
    pub prompt: Option<String>,
    pub context_tag: Option<String>,
}

impl DialogMatcher {
    pub fn any(kind: DialogKind) -> Self {
        Self {
            kind,
            window_id: None,
            title: None,
            prompt: None,
            context_tag: None,
        }
    }

    fn matches(&self, request: &DialogRequest) -> bool {
        self.kind == request.kind
            && self
                .window_id
                .is_none_or(|window_id| window_id == request.window_id)
            && self
                .title
                .as_ref()
                .is_none_or(|title| title == &request.title)
            && self
                .prompt
                .as_ref()
                .is_none_or(|prompt| request.prompt.as_ref() == Some(prompt))
            && self
                .context_tag
                .as_ref()
                .is_none_or(|context_tag| request.context_tag.as_ref() == Some(context_tag))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogScriptEntry {
    pub matcher: DialogMatcher,
    pub outcome: DialogOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogOutcome {
    SaveFile {
        result: Option<PathBuf>,
    },
    OpenFile {
        result: Option<PathBuf>,
    },
    ProfileSelection {
        chosen_profile_name: Option<String>,
        create_new_requested: bool,
        user_cancelled: bool,
    },
    Input {
        text: Option<String>,
    },
    ExcludePatterns {
        saved: bool,
        patterns: String,
    },
    Form {
        confirmed: bool,
        field_values: Vec<crate::FormFieldValue>,
    },
    FolderPicker {
        path: Option<PathBuf>,
    },
    MessageBox,
}

impl DialogOutcome {
    fn kind(&self) -> DialogKind {
        match self {
            Self::SaveFile { .. } => DialogKind::SaveFile,
            Self::OpenFile { .. } => DialogKind::OpenFile,
            Self::ProfileSelection { .. } => DialogKind::ProfileSelection,
            Self::Input { .. } => DialogKind::Input,
            Self::ExcludePatterns { .. } => DialogKind::ExcludePatterns,
            Self::Form { .. } => DialogKind::Form,
            Self::FolderPicker { .. } => DialogKind::FolderPicker,
            Self::MessageBox => DialogKind::MessageBox,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogRequest {
    pub kind: DialogKind,
    pub window_id: WindowId,
    pub title: String,
    pub prompt: Option<String>,
    pub context_tag: Option<String>,
    pub details: DialogRequestDetails,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogRequestDetails {
    SaveFile {
        default_filename: String,
        filter_spec: String,
        initial_dir: Option<PathBuf>,
    },
    OpenFile {
        filter_spec: String,
        initial_dir: Option<PathBuf>,
    },
    ProfileSelection {
        available_profiles: Vec<String>,
    },
    Input {
        default_text: Option<String>,
    },
    ExcludePatterns {
        patterns: String,
    },
    Form {
        form: crate::FormDialogDescriptor,
    },
    MessageBox {
        message: String,
        severity: MessageSeverity,
    },
    FolderPicker {
        initial_dir: Option<PathBuf>,
    },
}

const HEADLESS_PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, Deserialize)]
struct ProtocolRequestEnvelope {
    request_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ProtocolRequest {
    Action {
        request_id: u64,
        #[serde(flatten)]
        action: ProtocolActionRequest,
    },
    /// Replaces the ordered dialog responder script used by later `Show*Dialog` commands.
    /// Install this before the action that opens the dialog.
    SetDialogResponder {
        request_id: u64,
        script: Vec<ProtocolDialogScriptEntry>,
    },
    Snapshot {
        request_id: u64,
    },
    WaitFor {
        request_id: u64,
        label: String,
        timeout_ms: u64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ProtocolActionRequest {
    Click {
        window_id: usize,
        control_id: i32,
    },
    SetText {
        window_id: usize,
        control_id: i32,
        text: String,
    },
    SelectRow {
        window_id: usize,
        control_id: i32,
        item_id: u64,
    },
    SelectCombo {
        window_id: usize,
        control_id: i32,
        index: usize,
    },
    SelectTab {
        window_id: usize,
        control_id: i32,
        index: usize,
    },
    Toggle {
        window_id: usize,
        control_id: i32,
    },
    SelectRadio {
        window_id: usize,
        control_id: i32,
    },
    SelectTree {
        window_id: usize,
        control_id: i32,
        item_id: u64,
    },
    ToggleTree {
        window_id: usize,
        control_id: i32,
        item_id: u64,
    },
    ClickMenu {
        window_id: usize,
        action_id: u32,
    },
    Scroll {
        window_id: usize,
        control_id: i32,
        vertical_pos: u32,
        horizontal_pos: u32,
    },
}

#[derive(Debug, Deserialize)]
struct ProtocolDialogScriptEntry {
    matcher: ProtocolDialogMatcher,
    outcome: ProtocolDialogOutcome,
}

#[derive(Debug, Deserialize)]
struct ProtocolDialogMatcher {
    kind: String,
    window_id: Option<usize>,
    title: Option<String>,
    prompt: Option<String>,
    context_tag: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProtocolDialogOutcome {
    SaveFile {
        path: Option<String>,
    },
    OpenFile {
        path: Option<String>,
    },
    ProfileSelection {
        chosen_profile_name: Option<String>,
        create_new_requested: bool,
        user_cancelled: bool,
    },
    Input {
        text: Option<String>,
    },
    ExcludePatterns {
        saved: bool,
        patterns: String,
    },
    Form {
        confirmed: bool,
        field_values: Vec<ProtocolFormFieldValue>,
    },
    FolderPicker {
        path: Option<String>,
    },
    MessageBox,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProtocolFormFieldValue {
    Text { field_id: String, value: String },
    CheckBox { field_id: String, checked: bool },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ProtocolResponse {
    Hello {
        protocol_version: u32,
    },
    Snapshot {
        request_id: u64,
        model: ProtocolSnapshot,
    },
    Ok {
        request_id: u64,
    },
    Error {
        request_id: Option<u64>,
        message: String,
    },
    Marker {
        label: String,
    },
    Bye,
}

pub struct HeadlessHarness {
    backend: HeadlessBackend,
    handler: Option<SharedHandler>,
    ui_state_provider: Option<SharedProvider>,
    quit_notified: bool,
}

impl HeadlessHarness {
    pub fn new(app_name: impl Into<String>) -> Self {
        Self {
            backend: HeadlessBackend::new(app_name.into()),
            handler: None,
            ui_state_provider: None,
            quit_notified: false,
        }
    }

    pub fn create_window(&mut self, config: WindowConfig<'_>) -> PlatformResult<WindowId> {
        Ok(self.backend.create_window(config))
    }

    pub fn start(
        &mut self,
        handler: SharedHandler,
        ui_state_provider: SharedProvider,
        initial_commands: Vec<PlatformCommand>,
    ) -> PlatformResult<()> {
        self.handler = Some(handler);
        self.ui_state_provider = Some(ui_state_provider);
        self.backend.command_queue.extend(initial_commands);
        self.pump()
    }

    pub fn pump(&mut self) -> PlatformResult<()> {
        loop {
            let mut made_progress = false;

            while let Some(command) = self.backend.command_queue.pop_front() {
                made_progress = true;
                self.backend.execute_platform_command(command)?;
                if self.backend.quitting {
                    self.backend.command_queue.clear();
                    self.backend.follow_up_events.clear();
                    break;
                }
            }

            while let Some(event) = self.backend.follow_up_events.pop_front() {
                made_progress = true;
                self.deliver_event(event)?;
                if self.backend.quitting {
                    self.backend.command_queue.clear();
                    self.backend.follow_up_events.clear();
                    break;
                }
            }

            made_progress |= self.drain_handler_commands()?;

            if !made_progress || self.backend.quitting {
                break;
            }
        }

        self.finalize_quit()?;
        Ok(())
    }

    pub fn wait_for(&mut self, label: &str, timeout: Duration) -> PlatformResult<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.backend.markers.iter().any(|marker| marker == label) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(PlatformError::OperationFailed(format!(
                    "Timed out waiting for checkpoint marker '{label}'"
                )));
            }
            self.pump()?;
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Repeatedly pumps the UI until `predicate` returns `true` for the current snapshot.
    ///
    /// This is in-process only. External `--headless` protocol clients cannot send a Rust
    /// closure, so they must poll snapshots themselves.
    pub fn wait_until<F>(&mut self, predicate: F, timeout: Duration) -> PlatformResult<()>
    where
        F: Fn(&Value) -> bool,
    {
        let deadline = Instant::now() + timeout;
        loop {
            let snapshot = self.snapshot_value()?;
            if predicate(&snapshot) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(PlatformError::OperationFailed(
                    "Timed out waiting for headless condition".into(),
                ));
            }
            self.pump()?;
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn snapshot(&self) -> PlatformResult<String> {
        serde_json::to_string_pretty(&self.backend.snapshot()).map_err(|err| {
            PlatformError::OperationFailed(format!("Failed to serialize headless snapshot: {err}"))
        })
    }

    pub fn run_protocol<R, W>(&mut self, mut reader: R, mut writer: W) -> PlatformResult<()>
    where
        R: BufRead,
        W: Write,
    {
        Self::write_protocol_line(
            &mut writer,
            &ProtocolResponse::Hello {
                protocol_version: HEADLESS_PROTOCOL_VERSION,
            },
        )?;
        Self::flush_protocol_writer(&mut writer)?;

        let mut marker_cursor = self.backend.markers.len();
        let mut line = String::new();

        loop {
            line.clear();
            let bytes_read = reader.read_line(&mut line).map_err(|err| {
                PlatformError::OperationFailed(format!(
                    "Failed to read headless protocol input: {err}"
                ))
            })?;
            if bytes_read == 0 {
                self.emit_protocol_markers(&mut writer, &mut marker_cursor)?;
                Self::write_protocol_line(&mut writer, &ProtocolResponse::Bye)?;
                Self::flush_protocol_writer(&mut writer)?;
                return Ok(());
            }

            let raw_value = match serde_json::from_str::<Value>(&line) {
                Ok(value) => value,
                Err(err) => {
                    Self::respond_protocol_error(
                        &mut writer,
                        None,
                        format!("Malformed headless protocol request: {err}"),
                    )?;
                    continue;
                }
            };

            let envelope =
                match serde_json::from_value::<ProtocolRequestEnvelope>(raw_value.clone()) {
                    Ok(envelope) => envelope,
                    Err(err) => {
                        Self::respond_protocol_error(
                            &mut writer,
                            None,
                            format!("Malformed headless protocol request: {err}"),
                        )?;
                        continue;
                    }
                };
            let request_id = envelope.request_id;

            let request = match serde_json::from_value::<ProtocolRequest>(raw_value) {
                Ok(request) => request,
                Err(err) => {
                    Self::respond_protocol_error(
                        &mut writer,
                        request_id,
                        format!("Malformed headless protocol request: {err}"),
                    )?;
                    continue;
                }
            };

            let response_result: Result<ProtocolResponse, (Option<u64>, PlatformError)> =
                match request {
                    ProtocolRequest::Action { request_id, action } => {
                        match self.execute_protocol_action(action) {
                            Ok(()) => Ok(ProtocolResponse::Ok { request_id }),
                            Err(err) => Err((Some(request_id), err)),
                        }
                    }
                    ProtocolRequest::Snapshot { request_id } => match self.pump() {
                        Ok(()) => Ok(ProtocolResponse::Snapshot {
                            request_id,
                            model: self.backend.protocol_snapshot(),
                        }),
                        Err(err) => Err((Some(request_id), err)),
                    },
                    ProtocolRequest::WaitFor {
                        request_id,
                        label,
                        timeout_ms,
                    } => match self.wait_for_protocol_marker(
                        &label,
                        Duration::from_millis(timeout_ms),
                        marker_cursor,
                    ) {
                        Ok(()) => Ok(ProtocolResponse::Ok { request_id }),
                        Err(err) => Err((Some(request_id), err)),
                    },
                    ProtocolRequest::SetDialogResponder { request_id, script } => {
                        match self.set_protocol_dialog_responder(script) {
                            Ok(()) => Ok(ProtocolResponse::Ok { request_id }),
                            Err(err) => Err((Some(request_id), err)),
                        }
                    }
                };

            match response_result {
                Ok(response) => {
                    self.emit_protocol_markers(&mut writer, &mut marker_cursor)?;
                    Self::write_protocol_line(&mut writer, &response)?;
                    if self.finish_protocol_response_group(&mut writer)? {
                        return Ok(());
                    }
                }
                Err((request_id, err)) => {
                    self.emit_protocol_markers(&mut writer, &mut marker_cursor)?;
                    Self::write_protocol_line(
                        &mut writer,
                        &ProtocolResponse::Error {
                            request_id,
                            message: err.to_string(),
                        },
                    )?;
                    if self.finish_protocol_response_group(&mut writer)? {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Installs the ordered dialog responder script used by subsequent `Show*Dialog` commands.
    pub fn set_dialog_responder(&mut self, script: Vec<DialogScriptEntry>) {
        self.backend.dialog_responder = script.into();
    }

    fn snapshot_value(&self) -> PlatformResult<Value> {
        serde_json::to_value(self.backend.snapshot()).map_err(|err| {
            PlatformError::OperationFailed(format!("Failed to serialize headless snapshot: {err}"))
        })
    }

    fn emit_protocol_markers<W: Write>(
        &self,
        writer: &mut W,
        marker_cursor: &mut usize,
    ) -> PlatformResult<()> {
        while *marker_cursor < self.backend.markers.len() {
            let label = self.backend.markers[*marker_cursor].clone();
            Self::write_protocol_line(writer, &ProtocolResponse::Marker { label })?;
            *marker_cursor += 1;
        }
        Ok(())
    }

    fn write_protocol_line<W: Write, T: Serialize>(
        writer: &mut W,
        value: &T,
    ) -> PlatformResult<()> {
        serde_json::to_writer(&mut *writer, value).map_err(|err| {
            PlatformError::OperationFailed(format!(
                "Failed to serialize headless protocol line: {err}"
            ))
        })?;
        writer.write_all(b"\n").map_err(|err| {
            PlatformError::OperationFailed(format!("Failed to write headless protocol line: {err}"))
        })?;
        Ok(())
    }

    fn respond_protocol_error<W: Write>(
        writer: &mut W,
        request_id: Option<u64>,
        message: String,
    ) -> PlatformResult<()> {
        Self::write_protocol_line(
            writer,
            &ProtocolResponse::Error {
                request_id,
                message,
            },
        )?;
        Self::flush_protocol_writer(writer)
    }

    fn finish_protocol_response_group<W: Write>(&self, writer: &mut W) -> PlatformResult<bool> {
        if self.backend.quitting {
            Self::write_protocol_line(writer, &ProtocolResponse::Bye)?;
            Self::flush_protocol_writer(writer)?;
            return Ok(true);
        }
        Self::flush_protocol_writer(writer)?;
        Ok(false)
    }

    fn flush_protocol_writer<W: Write>(writer: &mut W) -> PlatformResult<()> {
        writer.flush().map_err(|err| {
            PlatformError::OperationFailed(format!(
                "Failed to flush headless protocol output: {err}"
            ))
        })
    }

    fn wait_for_protocol_marker(
        &mut self,
        label: &str,
        timeout: Duration,
        marker_cursor: usize,
    ) -> PlatformResult<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if self
                .backend
                .markers
                .get(marker_cursor..)
                .unwrap_or(&[])
                .iter()
                .any(|marker| marker == label)
            {
                return Ok(());
            }
            if self.backend.quitting {
                return Err(PlatformError::OperationFailed(format!(
                    "Headless protocol terminated while waiting for checkpoint marker '{label}'"
                )));
            }
            if Instant::now() >= deadline {
                return Err(PlatformError::OperationFailed(format!(
                    "Timed out waiting for checkpoint marker '{label}'"
                )));
            }
            self.pump()?;
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn execute_protocol_action(&mut self, action: ProtocolActionRequest) -> PlatformResult<()> {
        match action {
            ProtocolActionRequest::Click {
                window_id,
                control_id,
            } => self.click(WindowId::new(window_id), ControlId::new(control_id)),
            ProtocolActionRequest::SetText {
                window_id,
                control_id,
                text,
            } => self.set_text(WindowId::new(window_id), ControlId::new(control_id), text),
            ProtocolActionRequest::SelectRow {
                window_id,
                control_id,
                item_id,
            } => self.select_row(
                WindowId::new(window_id),
                ControlId::new(control_id),
                ListBoxItemId::new(item_id),
            ),
            ProtocolActionRequest::SelectCombo {
                window_id,
                control_id,
                index,
            } => self.select_combo(WindowId::new(window_id), ControlId::new(control_id), index),
            ProtocolActionRequest::SelectTab {
                window_id,
                control_id,
                index,
            } => self.select_tab(WindowId::new(window_id), ControlId::new(control_id), index),
            ProtocolActionRequest::Toggle {
                window_id,
                control_id,
            } => self.toggle(WindowId::new(window_id), ControlId::new(control_id)),
            ProtocolActionRequest::SelectRadio {
                window_id,
                control_id,
            } => self.select_radio(WindowId::new(window_id), ControlId::new(control_id)),
            ProtocolActionRequest::SelectTree {
                window_id,
                control_id,
                item_id,
            } => self.select_tree_item(
                WindowId::new(window_id),
                ControlId::new(control_id),
                TreeItemId::new(item_id),
            ),
            ProtocolActionRequest::ToggleTree {
                window_id,
                control_id,
                item_id,
            } => self.toggle_tree_item(
                WindowId::new(window_id),
                ControlId::new(control_id),
                TreeItemId::new(item_id),
            ),
            ProtocolActionRequest::ClickMenu {
                window_id,
                action_id,
            } => self.click_menu_action(WindowId::new(window_id), MenuActionId::new(action_id)),
            ProtocolActionRequest::Scroll {
                window_id,
                control_id,
                vertical_pos,
                horizontal_pos,
            } => self.scroll(
                WindowId::new(window_id),
                ControlId::new(control_id),
                vertical_pos,
                horizontal_pos,
            ),
        }
    }

    /// Injects a raw `AppEvent` into the harness.
    ///
    /// This is the in-process escape hatch for tests and host-side code that already has a
    /// concrete native event. It bypasses semantic-action validation but still routes through the
    /// normal follow-up queue and pump.
    pub fn inject_raw(&mut self, event: AppEvent) -> PlatformResult<()> {
        self.enqueue_follow_up_event(event);
        self.pump()
    }

    pub fn set_text(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        text: impl Into<String>,
    ) -> PlatformResult<()> {
        let text = text.into();
        self.backend
            .validate_visible_enabled_input(window_id, control_id)?;
        self.backend
            .set_input_text(window_id, control_id, text.clone())?;
        self.enqueue_follow_up_event(AppEvent::InputTextChanged {
            window_id,
            control_id,
            text,
        });
        self.pump()
    }

    pub fn select_row(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: ListBoxItemId,
    ) -> PlatformResult<()> {
        self.backend
            .select_list_box_item(window_id, control_id, item_id)?;
        self.enqueue_follow_up_event(AppEvent::ListBoxItemSelectionChanged {
            window_id,
            control_id,
            item_id,
        });
        self.pump()
    }

    pub fn select_combo(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        selected_index: usize,
    ) -> PlatformResult<()> {
        self.backend
            .validate_visible_enabled_combo(window_id, control_id)?;
        self.backend
            .set_combo_box_selection(window_id, control_id, Some(selected_index))?;
        self.enqueue_follow_up_event(AppEvent::ComboBoxSelectionChanged {
            window_id,
            control_id,
            selected_index: Some(selected_index),
        });
        self.pump()
    }

    pub fn select_tab(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        selected_index: usize,
    ) -> PlatformResult<()> {
        self.backend
            .validate_visible_enabled_tab(window_id, control_id)?;
        self.backend
            .set_tab_bar_selection(window_id, control_id, selected_index)?;
        self.enqueue_follow_up_event(AppEvent::TabBarSelectionChanged {
            window_id,
            control_id,
            selected_index,
        });
        self.pump()
    }

    pub fn toggle(&mut self, window_id: WindowId, control_id: ControlId) -> PlatformResult<()> {
        let event = self.backend.toggle_control(window_id, control_id)?;
        self.enqueue_follow_up_event(event);
        self.pump()
    }

    pub fn select_radio(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<()> {
        self.backend.select_radio_button(window_id, control_id)?;
        self.enqueue_follow_up_event(AppEvent::RadioButtonSelected {
            window_id,
            control_id,
        });
        self.pump()
    }

    pub fn click(&mut self, window_id: WindowId, control_id: ControlId) -> PlatformResult<()> {
        self.backend
            .validate_visible_enabled_button(window_id, control_id)?;
        self.enqueue_follow_up_event(AppEvent::ButtonClicked {
            window_id,
            control_id,
        });
        self.pump()
    }

    pub fn select_tree_item(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: TreeItemId,
    ) -> PlatformResult<()> {
        self.backend
            .validate_visible_enabled_tree_view(window_id, control_id)?;
        let event = self
            .backend
            .set_tree_view_selection(window_id, control_id, item_id)?;
        self.enqueue_follow_up_event(event);
        self.pump()
    }

    pub fn toggle_tree_item(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: TreeItemId,
    ) -> PlatformResult<()> {
        self.backend
            .validate_visible_enabled_tree_view(window_id, control_id)?;
        if let Some(new_state) = self
            .backend
            .toggle_tree_item(window_id, control_id, item_id)?
        {
            self.enqueue_follow_up_event(AppEvent::TreeViewItemToggledByUser {
                window_id,
                item_id,
                new_state,
            });
        }
        self.pump()
    }

    pub fn click_menu_action(
        &mut self,
        window_id: WindowId,
        action_id: MenuActionId,
    ) -> PlatformResult<()> {
        self.backend.validate_window_visible(window_id)?;
        self.backend.click_menu_action(window_id, action_id)?;
        self.enqueue_follow_up_event(AppEvent::MenuActionClicked { action_id });
        self.pump()
    }

    pub fn scroll(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        vertical_pos: u32,
        horizontal_pos: u32,
    ) -> PlatformResult<()> {
        self.backend
            .validate_visible_enabled_scrollable(window_id, control_id)?;
        self.backend
            .set_scroll_position(window_id, control_id, vertical_pos, horizontal_pos)?;
        self.enqueue_follow_up_event(AppEvent::ControlScrolled {
            window_id,
            control_id,
            vertical_pos,
            horizontal_pos,
        });
        self.pump()
    }

    fn enqueue_follow_up_event(&mut self, event: AppEvent) {
        self.backend.follow_up_events.push_back(event);
    }

    fn deliver_event(&mut self, event: AppEvent) -> PlatformResult<()> {
        let Some(handler) = &self.handler else {
            return Err(PlatformError::OperationFailed(
                "Headless harness has no event handler installed".into(),
            ));
        };

        let mut guard = handler.lock().map_err(|err| {
            PlatformError::OperationFailed(format!("Failed to lock headless event handler: {err}"))
        })?;
        guard.handle_event(event);
        Ok(())
    }

    fn drain_handler_commands(&mut self) -> PlatformResult<bool> {
        let Some(handler) = &self.handler else {
            return Ok(false);
        };
        let mut moved_any = false;

        loop {
            let next_command = {
                let mut guard = handler.lock().map_err(|err| {
                    PlatformError::OperationFailed(format!(
                        "Failed to lock headless event handler for command drain: {err}"
                    ))
                })?;
                guard.try_dequeue_command()
            };

            match next_command {
                Some(command) => {
                    self.backend.command_queue.push_back(command);
                    moved_any = true;
                }
                None => break,
            }
        }

        Ok(moved_any)
    }

    fn finalize_quit(&mut self) -> PlatformResult<()> {
        if self.backend.quitting && !self.quit_notified {
            if let Some(handler) = &self.handler {
                let mut guard = handler.lock().map_err(|err| {
                    PlatformError::OperationFailed(format!(
                        "Failed to lock headless event handler for on_quit: {err}"
                    ))
                })?;
                guard.on_quit();
            }
            self.quit_notified = true;
        }
        Ok(())
    }

    fn set_protocol_dialog_responder(
        &mut self,
        script: Vec<ProtocolDialogScriptEntry>,
    ) -> PlatformResult<()> {
        let mut script = script
            .into_iter()
            .map(ProtocolDialogScriptEntry::into_runtime)
            .collect::<PlatformResult<Vec<_>>>()?;
        // Message boxes have no completion event and never consult the responder queue.
        script.retain(|entry| entry.matcher.kind != DialogKind::MessageBox);
        self.set_dialog_responder(script);
        Ok(())
    }
}

#[derive(Debug)]
struct HeadlessBackend {
    app_name: String,
    next_window_id: usize,
    windows: BTreeMap<usize, WindowState>,
    // Retained to mirror native DefineStyle state; Phase 2b snapshots only expose applied style ids.
    styles: HashMap<StyleId, ControlStyle>,
    markers: Vec<String>,
    dialog_requests: Vec<DialogRequest>,
    dialog_responder: VecDeque<DialogScriptEntry>,
    command_queue: VecDeque<PlatformCommand>,
    follow_up_events: VecDeque<AppEvent>,
    quitting: bool,
}

impl HeadlessBackend {
    fn new(app_name: String) -> Self {
        Self {
            app_name,
            next_window_id: 1,
            windows: BTreeMap::new(),
            styles: HashMap::new(),
            markers: Vec::new(),
            dialog_requests: Vec::new(),
            dialog_responder: VecDeque::new(),
            command_queue: VecDeque::new(),
            follow_up_events: VecDeque::new(),
            quitting: false,
        }
    }

    fn snapshot(&self) -> HeadlessSnapshot {
        HeadlessSnapshot {
            app_name: self.app_name.clone(),
            quitting: self.quitting,
            markers: self.markers.clone(),
            dialog_requests: self
                .dialog_requests
                .iter()
                .map(DialogRequestSnapshot::from)
                .collect(),
            windows: self.windows.values().map(WindowState::snapshot).collect(),
        }
    }

    fn protocol_snapshot(&self) -> ProtocolSnapshot {
        ProtocolSnapshot {
            app_name: self.app_name.clone(),
            dialog_requests: self
                .dialog_requests
                .iter()
                .map(DialogRequestSnapshot::from)
                .collect(),
            windows: self.windows.values().map(WindowState::snapshot).collect(),
        }
    }

    fn create_window(&mut self, config: WindowConfig<'_>) -> WindowId {
        let window_id = WindowId::new(self.next_window_id);
        self.next_window_id += 1;
        self.windows.insert(
            window_id.raw(),
            WindowState::new(window_id, config.title, config.width, config.height),
        );
        window_id
    }

    fn execute_platform_command(&mut self, command: PlatformCommand) -> PlatformResult<()> {
        match command {
            PlatformCommand::SetWindowTitle { window_id, title } => {
                self.with_window_mut(window_id, |window| {
                    window.ensure_open()?;
                    window.title = title;
                    Ok(())
                })
            }
            PlatformCommand::ShowWindow { window_id } => {
                self.with_window_mut(window_id, |window| {
                    window.ensure_not_closed()?;
                    window.shown = true;
                    Ok(())
                })
            }
            PlatformCommand::CloseWindow { window_id } => {
                self.with_window_mut(window_id, |window| {
                    window.closed = true;
                    window.shown = false;
                    Ok(())
                })
            }
            PlatformCommand::CreateButton {
                window_id,
                parent_control_id,
                control_id,
                text,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::Button { text },
            ),
            PlatformCommand::CreatePanel {
                window_id,
                parent_control_id,
                control_id,
            } => self.create_control(window_id, parent_control_id, control_id, ControlKind::Panel),
            PlatformCommand::CreateLabel {
                window_id,
                parent_control_id,
                control_id,
                initial_text,
                class,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::Label {
                    text: initial_text,
                    class,
                    severity: MessageSeverity::None,
                },
            ),
            PlatformCommand::CreateInput {
                window_id,
                parent_control_id,
                control_id,
                initial_text,
                read_only,
                multiline,
                vertical_scroll,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::Input {
                    text: initial_text,
                    read_only,
                    multiline,
                    vertical_scroll,
                },
            ),
            PlatformCommand::CreateRichEdit {
                window_id,
                parent_control_id,
                control_id,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::RichEdit {
                    text: String::new(),
                },
            ),
            PlatformCommand::SetInputText {
                window_id,
                control_id,
                text,
            }
            | PlatformCommand::SetControlText {
                window_id,
                control_id,
                text,
            }
            | PlatformCommand::SetViewerContent {
                window_id,
                control_id,
                text,
            } => self.set_control_text(window_id, control_id, text),
            PlatformCommand::SetRichEditContent {
                window_id,
                control_id,
                rtf_text,
            } => self.set_rich_edit_content(window_id, control_id, rtf_text),
            PlatformCommand::UpdateLabelText {
                window_id,
                control_id,
                text,
                severity,
            } => self.update_label_text(window_id, control_id, text, severity),
            PlatformCommand::SetControlEnabled {
                window_id,
                control_id,
                enabled,
            } => self.with_control_mut(window_id, control_id, |control| {
                control.enabled = enabled;
                Ok(())
            }),
            PlatformCommand::DefineLayout { window_id, rules } => {
                self.define_layout(window_id, rules)
            }
            PlatformCommand::QuitApplication => {
                self.quitting = true;
                Ok(())
            }
            PlatformCommand::SignalMainWindowUISetupComplete { window_id } => {
                self.ensure_window_exists(window_id)?;
                self.follow_up_events
                    .push_back(AppEvent::MainWindowUISetupComplete { window_id });
                Ok(())
            }
            PlatformCommand::Checkpoint { label } => {
                self.markers.push(label);
                Ok(())
            }
            PlatformCommand::CreateListBox {
                window_id,
                parent_control_id,
                control_id,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::ListBox {
                    items: Vec::new(),
                    selected_item_id: None,
                    badge_column_width: 0,
                    density: ListBoxRowDensity::Expanded,
                },
            ),
            PlatformCommand::PopulateListBox {
                window_id,
                control_id,
                items,
                badge_column_width,
            } => self.populate_list_box(window_id, control_id, items, badge_column_width),
            PlatformCommand::SetListBoxRowDensity {
                window_id,
                control_id,
                density,
            } => self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
                ControlKind::ListBox {
                    density: current, ..
                } => {
                    *current = density;
                    Ok(())
                }
                _ => Err(Self::unsupported_control_kind(
                    "SetListBoxRowDensity",
                    "ListBox",
                )),
            }),
            PlatformCommand::SetListBoxSelection {
                window_id,
                control_id,
                item_id,
            } => self.set_list_box_selection(window_id, control_id, item_id),
            PlatformCommand::CreateComboBox {
                window_id,
                parent_control_id,
                control_id,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::ComboBox {
                    items: Vec::new(),
                    selected_index: None,
                },
            ),
            PlatformCommand::SetComboBoxItems {
                window_id,
                control_id,
                items,
            } => self.set_combo_box_items(window_id, control_id, items),
            PlatformCommand::SetComboBoxSelection {
                window_id,
                control_id,
                selected_index,
            } => self.set_combo_box_selection(window_id, control_id, selected_index),
            PlatformCommand::CreateRadioButton {
                window_id,
                parent_control_id,
                control_id,
                text,
                group_start,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::RadioButton {
                    text,
                    checked: false,
                    group_start,
                },
            ),
            PlatformCommand::SetRadioButtonChecked {
                window_id,
                control_id,
                checked,
            } => self.set_radio_button_checked(window_id, control_id, checked),
            PlatformCommand::CreateCheckBox {
                window_id,
                parent_control_id,
                control_id,
                text,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::CheckBox {
                    text,
                    checked: false,
                },
            ),
            PlatformCommand::SetCheckBoxChecked {
                window_id,
                control_id,
                checked,
            } => self.set_check_box_checked(window_id, control_id, checked),
            PlatformCommand::CreateTabBar {
                window_id,
                control_id,
                parent_control_id,
                items,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::TabBar {
                    items,
                    selected_index: None,
                },
            ),
            PlatformCommand::SetTabBarItems {
                window_id,
                control_id,
                items,
            } => self.set_tab_bar_items(window_id, control_id, items),
            PlatformCommand::SetTabBarSelection {
                window_id,
                control_id,
                selected_index,
            } => self.set_tab_bar_selection(window_id, control_id, selected_index),
            PlatformCommand::CreateToggleSwitch {
                window_id,
                parent_control_id,
                control_id,
                label,
                checked,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::ToggleSwitch { label, checked },
            ),
            PlatformCommand::SetToggleSwitchState {
                window_id,
                control_id,
                checked,
            } => self.set_toggle_switch_state(window_id, control_id, checked),
            PlatformCommand::CreateProgressBar {
                window_id,
                parent_control_id,
                control_id,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::ProgressBar {
                    min: 0,
                    max: 100,
                    position: 0,
                },
            ),
            PlatformCommand::SetProgressBarRange {
                window_id,
                control_id,
                min,
                max,
            } => self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
                ControlKind::ProgressBar {
                    min: current_min,
                    max: current_max,
                    position,
                } => {
                    *current_min = min;
                    *current_max = max;
                    *position = (*position).clamp(min, max);
                    Ok(())
                }
                _ => Err(Self::unsupported_control_kind(
                    "SetProgressBarRange",
                    "ProgressBar",
                )),
            }),
            PlatformCommand::SetProgressBarPosition {
                window_id,
                control_id,
                position,
            } => self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
                ControlKind::ProgressBar {
                    min,
                    max,
                    position: current_position,
                } => {
                    *current_position = position.clamp(*min, *max);
                    Ok(())
                }
                _ => Err(Self::unsupported_control_kind(
                    "SetProgressBarPosition",
                    "ProgressBar",
                )),
            }),
            PlatformCommand::CreateSplitter {
                window_id,
                parent_control_id,
                control_id,
                orientation,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::Splitter { orientation },
            ),
            PlatformCommand::SetFocus {
                window_id,
                control_id,
                select_all,
            } => self.with_window_and_control_mut(window_id, control_id, |window, control| {
                window.focused_control_id = Some(control_id);
                if select_all {
                    control.selected_all = true;
                }
                Ok(())
            }),
            PlatformCommand::CreateTreeView {
                window_id,
                parent_control_id,
                control_id,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::TreeView {
                    items: Vec::new(),
                    selected_item_id: None,
                },
            ),
            PlatformCommand::PopulateTreeView {
                window_id,
                control_id,
                items,
            } => self.populate_tree_view(window_id, control_id, items),
            PlatformCommand::UpdateTreeItemVisualState {
                window_id,
                control_id,
                item_id,
                new_state,
            } => self.update_tree_item_visual_state(window_id, control_id, item_id, new_state),
            PlatformCommand::UpdateTreeItemText {
                window_id,
                control_id,
                item_id,
                text,
            } => self.update_tree_item_text(window_id, control_id, item_id, text),
            PlatformCommand::CreateMainMenu {
                window_id,
                menu_items,
            } => self.create_main_menu(window_id, menu_items),
            PlatformCommand::CreateChart {
                window_id,
                parent_control_id,
                control_id,
            } => self.create_control(
                window_id,
                parent_control_id,
                control_id,
                ControlKind::Chart { data: None },
            ),
            PlatformCommand::SetChartData {
                window_id,
                control_id,
                data,
            } => self.set_chart_data(window_id, control_id, data),
            PlatformCommand::ExpandVisibleTreeItems {
                window_id,
                control_id,
            } => self.expand_tree_items(window_id, control_id),
            PlatformCommand::ExpandAllTreeItems {
                window_id,
                control_id,
            } => self.expand_tree_items(window_id, control_id),
            PlatformCommand::RedrawTreeItem {
                window_id,
                control_id,
                item_id,
            } => self.redraw_tree_item(window_id, control_id, item_id),
            PlatformCommand::SetTabBarStyle {
                window_id,
                control_id,
                background_color,
                text_color,
                accent_color,
                font,
            } => self.set_tab_bar_style(
                window_id,
                control_id,
                background_color,
                text_color,
                accent_color,
                font,
            ),
            PlatformCommand::DefineStyle { style_id, style } => self.define_style(style_id, style),
            PlatformCommand::ApplyStyleToControl {
                window_id,
                control_id,
                style_id,
            } => self.apply_style_to_control(window_id, control_id, style_id),
            PlatformCommand::SetToggleSwitchStyle {
                window_id,
                control_id,
                background,
                pill_off,
                pill_on,
                knob,
                text,
            } => self.set_toggle_switch_style(
                window_id,
                control_id,
                (background, pill_off, pill_on, knob, text),
            ),
            PlatformCommand::SetScrollPosition {
                window_id,
                control_id,
                vertical_pos,
                horizontal_pos,
            } => self.set_scroll_position(window_id, control_id, vertical_pos, horizontal_pos),
            PlatformCommand::SetTreeViewSelection {
                window_id,
                control_id,
                item_id,
            } => {
                let event = self.set_tree_view_selection(window_id, control_id, item_id)?;
                self.follow_up_events.push_back(event);
                Ok(())
            }
            PlatformCommand::ShowSaveFileDialog {
                window_id,
                title,
                default_filename,
                filter_spec,
                initial_dir,
            } => self.handle_save_file_dialog(
                window_id,
                title,
                default_filename,
                filter_spec,
                initial_dir,
            ),
            PlatformCommand::ShowOpenFileDialog {
                window_id,
                title,
                filter_spec,
                initial_dir,
            } => self.handle_open_file_dialog(window_id, title, filter_spec, initial_dir),
            PlatformCommand::ShowProfileSelectionDialog {
                window_id,
                available_profiles,
                title,
                prompt,
            } => self.handle_profile_selection_dialog(window_id, available_profiles, title, prompt),
            PlatformCommand::ShowInputDialog {
                window_id,
                title,
                prompt,
                default_text,
                context_tag,
            } => self.handle_input_dialog(window_id, title, prompt, default_text, context_tag),
            PlatformCommand::ShowExcludePatternsDialog {
                window_id,
                title,
                patterns,
            } => self.handle_exclude_patterns_dialog(window_id, title, patterns),
            PlatformCommand::ShowFormDialog { window_id, form } => {
                self.handle_form_dialog(window_id, form)
            }
            PlatformCommand::ShowMessageBox {
                window_id,
                title,
                message,
                severity,
            } => self.handle_message_box(window_id, title, message, severity),
            PlatformCommand::ShowFolderPickerDialog {
                window_id,
                title,
                initial_dir,
            } => self.handle_folder_picker_dialog(window_id, title, initial_dir),
        }
    }

    fn handle_save_file_dialog(
        &mut self,
        window_id: WindowId,
        title: String,
        default_filename: String,
        filter_spec: String,
        initial_dir: Option<PathBuf>,
    ) -> PlatformResult<()> {
        self.ensure_window_exists(window_id)?;
        let request = DialogRequest {
            kind: DialogKind::SaveFile,
            window_id,
            title,
            prompt: None,
            context_tag: None,
            details: DialogRequestDetails::SaveFile {
                default_filename,
                filter_spec,
                initial_dir,
            },
        };
        self.complete_dialog_request(request);
        Ok(())
    }

    fn handle_open_file_dialog(
        &mut self,
        window_id: WindowId,
        title: String,
        filter_spec: String,
        initial_dir: Option<PathBuf>,
    ) -> PlatformResult<()> {
        self.ensure_window_exists(window_id)?;
        let request = DialogRequest {
            kind: DialogKind::OpenFile,
            window_id,
            title,
            prompt: None,
            context_tag: None,
            details: DialogRequestDetails::OpenFile {
                filter_spec,
                initial_dir,
            },
        };
        self.complete_dialog_request(request);
        Ok(())
    }

    fn handle_profile_selection_dialog(
        &mut self,
        window_id: WindowId,
        available_profiles: Vec<String>,
        title: String,
        prompt: String,
    ) -> PlatformResult<()> {
        self.ensure_window_exists(window_id)?;
        let request = DialogRequest {
            kind: DialogKind::ProfileSelection,
            window_id,
            title,
            prompt: Some(prompt),
            context_tag: None,
            details: DialogRequestDetails::ProfileSelection { available_profiles },
        };
        self.complete_dialog_request(request);
        Ok(())
    }

    fn handle_input_dialog(
        &mut self,
        window_id: WindowId,
        title: String,
        prompt: String,
        default_text: Option<String>,
        context_tag: Option<String>,
    ) -> PlatformResult<()> {
        self.ensure_window_exists(window_id)?;
        let request = DialogRequest {
            kind: DialogKind::Input,
            window_id,
            title,
            prompt: Some(prompt),
            context_tag: context_tag.clone(),
            details: DialogRequestDetails::Input { default_text },
        };
        self.complete_dialog_request(request);
        Ok(())
    }

    fn handle_exclude_patterns_dialog(
        &mut self,
        window_id: WindowId,
        title: String,
        patterns: String,
    ) -> PlatformResult<()> {
        self.ensure_window_exists(window_id)?;
        let request = DialogRequest {
            kind: DialogKind::ExcludePatterns,
            window_id,
            title,
            prompt: None,
            context_tag: None,
            details: DialogRequestDetails::ExcludePatterns { patterns },
        };
        self.complete_dialog_request(request);
        Ok(())
    }

    fn handle_form_dialog(
        &mut self,
        window_id: WindowId,
        form: crate::FormDialogDescriptor,
    ) -> PlatformResult<()> {
        self.ensure_window_exists(window_id)?;
        let title = form.title.clone();
        let context_tag = Some(form.context_tag.clone());
        let request = DialogRequest {
            kind: DialogKind::Form,
            window_id,
            title,
            prompt: None,
            context_tag: context_tag.clone(),
            details: DialogRequestDetails::Form { form },
        };
        self.complete_dialog_request(request);
        Ok(())
    }

    fn handle_message_box(
        &mut self,
        window_id: WindowId,
        title: String,
        message: String,
        severity: MessageSeverity,
    ) -> PlatformResult<()> {
        self.ensure_window_exists(window_id)?;
        let request = DialogRequest {
            kind: DialogKind::MessageBox,
            window_id,
            title,
            prompt: Some(message.clone()),
            context_tag: None,
            details: DialogRequestDetails::MessageBox { message, severity },
        };
        self.record_dialog_request(request);
        Ok(())
    }

    fn handle_folder_picker_dialog(
        &mut self,
        window_id: WindowId,
        title: String,
        initial_dir: Option<PathBuf>,
    ) -> PlatformResult<()> {
        self.ensure_window_exists(window_id)?;
        let request = DialogRequest {
            kind: DialogKind::FolderPicker,
            window_id,
            title,
            prompt: None,
            context_tag: None,
            details: DialogRequestDetails::FolderPicker { initial_dir },
        };
        self.complete_dialog_request(request);
        Ok(())
    }

    fn record_dialog_request(&mut self, request: DialogRequest) {
        self.dialog_requests.push(request);
    }

    fn complete_dialog_request(&mut self, request: DialogRequest) {
        self.record_dialog_request(request.clone());
        let outcome = self.take_dialog_outcome_for(&request);
        if let Some(event) = self.dialog_completion_event(&request, outcome) {
            self.follow_up_events.push_back(event);
        }
    }

    fn take_dialog_outcome_for(&mut self, request: &DialogRequest) -> DialogOutcome {
        if let Some(front) = self.dialog_responder.front()
            && front.matcher.matches(request)
        {
            return self.dialog_responder.pop_front().unwrap().outcome;
        }
        Self::default_dialog_outcome(request)
    }

    fn default_dialog_outcome(request: &DialogRequest) -> DialogOutcome {
        match &request.details {
            DialogRequestDetails::SaveFile { .. } => DialogOutcome::SaveFile { result: None },
            DialogRequestDetails::OpenFile { .. } => DialogOutcome::OpenFile { result: None },
            DialogRequestDetails::ProfileSelection { .. } => DialogOutcome::ProfileSelection {
                chosen_profile_name: None,
                create_new_requested: false,
                user_cancelled: true,
            },
            DialogRequestDetails::Input { .. } => DialogOutcome::Input { text: None },
            DialogRequestDetails::ExcludePatterns { patterns } => DialogOutcome::ExcludePatterns {
                saved: false,
                patterns: patterns.clone(),
            },
            DialogRequestDetails::Form { form: _ } => DialogOutcome::Form {
                confirmed: false,
                field_values: Vec::new(),
            },
            DialogRequestDetails::MessageBox { .. } => DialogOutcome::MessageBox,
            DialogRequestDetails::FolderPicker { .. } => DialogOutcome::FolderPicker { path: None },
        }
    }

    fn dialog_completion_event(
        &self,
        request: &DialogRequest,
        outcome: DialogOutcome,
    ) -> Option<AppEvent> {
        match (request.kind.clone(), outcome) {
            (DialogKind::SaveFile, DialogOutcome::SaveFile { result }) => {
                Some(AppEvent::FileSaveDialogCompleted {
                    window_id: request.window_id,
                    result,
                })
            }
            (DialogKind::OpenFile, DialogOutcome::OpenFile { result }) => {
                Some(AppEvent::FileOpenProfileDialogCompleted {
                    window_id: request.window_id,
                    result,
                })
            }
            (
                DialogKind::ProfileSelection,
                DialogOutcome::ProfileSelection {
                    chosen_profile_name,
                    create_new_requested,
                    user_cancelled,
                },
            ) => Some(AppEvent::ProfileSelectionDialogCompleted {
                window_id: request.window_id,
                chosen_profile_name,
                create_new_requested,
                user_cancelled,
            }),
            (DialogKind::Input, DialogOutcome::Input { text }) => {
                Some(AppEvent::GenericInputDialogCompleted {
                    window_id: request.window_id,
                    text,
                    context_tag: request.context_tag.clone(),
                })
            }
            (DialogKind::ExcludePatterns, DialogOutcome::ExcludePatterns { saved, patterns }) => {
                Some(AppEvent::ExcludePatternsDialogCompleted {
                    window_id: request.window_id,
                    saved,
                    patterns,
                })
            }
            (
                DialogKind::Form,
                DialogOutcome::Form {
                    confirmed,
                    field_values,
                },
            ) => Some(AppEvent::FormDialogCompleted {
                window_id: request.window_id,
                context_tag: request.context_tag.clone().unwrap_or_default(),
                confirmed,
                field_values,
            }),
            (DialogKind::FolderPicker, DialogOutcome::FolderPicker { path }) => {
                Some(AppEvent::FolderPickerDialogCompleted {
                    window_id: request.window_id,
                    path,
                })
            }
            (DialogKind::MessageBox, DialogOutcome::MessageBox) => None,
            _ => None,
        }
    }

    fn unsupported_control_kind(command: &str, expected: &str) -> PlatformError {
        PlatformError::OperationFailed(format!(
            "{command} is only supported for {expected} controls"
        ))
    }

    fn define_layout(&mut self, window_id: WindowId, rules: Vec<LayoutRule>) -> PlatformResult<()> {
        Self::validate_layout_rules(&rules)?;
        self.with_window_mut(window_id, |window| {
            for rule in &rules {
                if !window.controls.contains_key(&rule.control_id.raw()) {
                    return Err(PlatformError::InvalidHandle(format!(
                        "Control ID {} not found for DefineLayout in window {window_id:?}",
                        rule.control_id.raw()
                    )));
                }
                if let Some(parent_id) = rule.parent_control_id
                    && !window.controls.contains_key(&parent_id.raw())
                {
                    return Err(PlatformError::InvalidHandle(format!(
                        "Parent control ID {} not found for DefineLayout in window {window_id:?}",
                        parent_id.raw()
                    )));
                }
            }
            window.layout_rules = rules;
            Ok(())
        })
    }

    fn validate_layout_rules(rules: &[LayoutRule]) -> PlatformResult<()> {
        let mut fill_by_parent: BTreeMap<Option<i32>, Vec<i32>> = BTreeMap::new();
        for rule in rules {
            match rule.dock_style {
                DockStyle::Top | DockStyle::Bottom | DockStyle::Left | DockStyle::Right => {
                    if rule.fixed_size.is_none() {
                        return Err(PlatformError::OperationFailed(format!(
                            "DefineLayout rejected: control {} uses {:?} without fixed_size. Docked edges require explicit fixed_size.",
                            rule.control_id.raw(),
                            rule.dock_style
                        )));
                    }
                    if let Some(size) = rule.fixed_size
                        && size < 0
                    {
                        return Err(PlatformError::OperationFailed(format!(
                            "DefineLayout rejected: control {} has negative fixed_size {} for {:?}.",
                            rule.control_id.raw(),
                            size,
                            rule.dock_style
                        )));
                    }
                }
                _ => {}
            }

            if rule.dock_style == DockStyle::Fill {
                fill_by_parent
                    .entry(rule.parent_control_id.map(|id| id.raw()))
                    .or_default()
                    .push(rule.control_id.raw());
            }
        }

        for (parent_id, fill_controls) in fill_by_parent {
            if fill_controls.len() > 1 {
                let parent_desc = parent_id
                    .map(|id| format!("control {id}"))
                    .unwrap_or_else(|| "main window".to_string());
                let control_ids = fill_controls
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(PlatformError::OperationFailed(format!(
                    "DefineLayout rejected: parent {parent_desc} has multiple DockStyle::Fill children ({control_ids}). CommanDuctUI supports exactly one Fill child per parent."
                )));
            }
        }

        Ok(())
    }

    fn populate_list_box(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        items: Vec<ListBoxItemDescriptor>,
        badge_column_width: u16,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::ListBox {
                items: current_items,
                selected_item_id,
                badge_column_width: current_badge_column_width,
                ..
            } => {
                *current_badge_column_width = badge_column_width;
                *current_items = items;
                if let Some(selected) = selected_item_id
                    && !current_items.iter().any(|item| item.id == *selected)
                {
                    *selected_item_id = None;
                }
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind("PopulateListBox", "ListBox")),
        })
    }

    fn set_list_box_selection(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: ListBoxItemId,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::ListBox {
                items,
                selected_item_id,
                ..
            } => {
                if !items.iter().any(|item| item.id == item_id) {
                    return Err(PlatformError::InvalidHandle(format!(
                        "ListBox item {item_id:?} not found for selection in window {window_id:?}"
                    )));
                }
                *selected_item_id = Some(item_id);
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind(
                "SetListBoxSelection",
                "ListBox",
            )),
        })
    }

    fn set_combo_box_items(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        items: Vec<String>,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::ComboBox {
                items: current_items,
                selected_index,
            } => {
                *current_items = items;
                if let Some(index) = selected_index
                    && *index >= current_items.len()
                {
                    *selected_index = None;
                }
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind(
                "SetComboBoxItems",
                "ComboBox",
            )),
        })
    }

    fn set_combo_box_selection(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        selected_index: Option<usize>,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::ComboBox { items, selected_index: current_selected } => {
                if let Some(index) = selected_index && index >= items.len() {
                    return Err(PlatformError::InvalidHandle(format!(
                        "ComboBox selection index {index} is out of bounds for control {} in window {window_id:?}",
                        control_id.raw()
                    )));
                }
                *current_selected = selected_index;
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind("SetComboBoxSelection", "ComboBox")),
        })
    }

    fn set_tab_bar_items(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        items: Vec<String>,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::TabBar {
                items: current_items,
                selected_index,
            } => {
                *current_items = items;
                if let Some(index) = selected_index
                    && *index >= current_items.len()
                {
                    *selected_index = current_items.len().checked_sub(1);
                }
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind("SetTabBarItems", "TabBar")),
        })
    }

    fn set_tab_bar_selection(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        selected_index: usize,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::TabBar { items, selected_index: current_selected } => {
                if selected_index >= items.len() {
                    return Err(PlatformError::InvalidHandle(format!(
                        "TabBar selection index {selected_index} is out of bounds for control {} in window {window_id:?}",
                        control_id.raw()
                    )));
                }
                *current_selected = Some(selected_index);
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind("SetTabBarSelection", "TabBar")),
        })
    }

    fn set_check_box_checked(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        checked: bool,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::CheckBox {
                checked: current, ..
            } => {
                *current = checked;
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind(
                "SetCheckBoxChecked",
                "CheckBox",
            )),
        })
    }

    fn set_radio_button_checked(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        checked: bool,
    ) -> PlatformResult<()> {
        let _parent =
            self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
                ControlKind::RadioButton {
                    checked: current, ..
                } => {
                    *current = checked;
                    Ok(control.parent_control_id)
                }
                _ => Err(Self::unsupported_control_kind(
                    "SetRadioButtonChecked",
                    "RadioButton",
                )),
            })?;
        if checked {
            self.clear_other_radio_buttons(window_id, control_id)?;
        }
        Ok(())
    }

    fn set_toggle_switch_state(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        checked: bool,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::ToggleSwitch {
                checked: current, ..
            } => {
                *current = checked;
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind(
                "SetToggleSwitchState",
                "ToggleSwitch",
            )),
        })
    }

    fn populate_tree_view(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        items: Vec<TreeItemDescriptor>,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::TreeView {
                items: current_items,
                selected_item_id,
            } => {
                *current_items = items.iter().map(TreeItemNode::from_descriptor).collect();
                if let Some(selected) = selected_item_id
                    && !tree_items_contain_id(current_items, *selected)
                {
                    *selected_item_id = None;
                }
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind(
                "PopulateTreeView",
                "TreeView",
            )),
        })
    }

    fn update_tree_item_visual_state(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: TreeItemId,
        new_state: CheckState,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::TreeView { items, .. } => {
                let item = tree_items_find_mut(items, item_id).ok_or_else(|| {
                    PlatformError::InvalidHandle(format!(
                        "TreeItemId {item_id:?} not found in window {window_id:?}"
                    ))
                })?;
                item.state = new_state;
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind(
                "UpdateTreeItemVisualState",
                "TreeView",
            )),
        })
    }

    fn update_tree_item_text(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: TreeItemId,
        text: String,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::TreeView { items, .. } => {
                let item = tree_items_find_mut(items, item_id).ok_or_else(|| {
                    PlatformError::InvalidHandle(format!(
                        "TreeItemId {item_id:?} not found in window {window_id:?}"
                    ))
                })?;
                item.text = text;
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind(
                "UpdateTreeItemText",
                "TreeView",
            )),
        })
    }

    fn set_tree_view_selection(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: TreeItemId,
    ) -> PlatformResult<AppEvent> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::TreeView {
                items,
                selected_item_id,
            } => {
                if !tree_items_contain_id(items, item_id) {
                    return Err(PlatformError::InvalidHandle(format!(
                        "TreeItemId {item_id:?} not found in window {window_id:?}"
                    )));
                }
                *selected_item_id = Some(item_id);
                Ok(AppEvent::TreeViewItemSelectionChanged { window_id, item_id })
            }
            _ => Err(Self::unsupported_control_kind(
                "SetTreeViewSelection",
                "TreeView",
            )),
        })
    }

    fn expand_tree_items(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::TreeView { items, .. } => {
                for item in items.iter_mut() {
                    item.expand_recursive();
                }
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind(
                "ExpandTreeItems",
                "TreeView",
            )),
        })
    }

    fn redraw_tree_item(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: TreeItemId,
    ) -> PlatformResult<()> {
        self.find_tree_item(window_id, control_id, item_id)?;
        Ok(())
    }

    fn create_main_menu(
        &mut self,
        window_id: WindowId,
        menu_items: Vec<crate::MenuItemConfig>,
    ) -> PlatformResult<()> {
        self.with_window_mut(window_id, |window| {
            window.ensure_not_closed()?;
            window.menu_items = menu_items.iter().map(MenuNode::from_config).collect();
            Ok(())
        })
    }

    fn set_chart_data(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        data: ChartDataPacket,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::Chart { data: current } => {
                *current = Some(ChartDataState::from_packet(data));
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind("SetChartData", "Chart")),
        })
    }

    fn set_tab_bar_style(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        _background_color: crate::Color,
        _text_color: crate::Color,
        _accent_color: crate::Color,
        _font: Option<crate::FontDescription>,
    ) -> PlatformResult<()> {
        let control = self.with_control_ref(window_id, control_id)?;
        match &control.kind {
            ControlKind::TabBar { .. } => Ok(()),
            _ => Err(Self::unsupported_control_kind("SetTabBarStyle", "TabBar")),
        }
    }

    fn define_style(&mut self, style_id: StyleId, style: ControlStyle) -> PlatformResult<()> {
        self.styles.insert(style_id, style);
        Ok(())
    }

    fn apply_style_to_control(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        style_id: StyleId,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| {
            control.style_id = Some(style_id.stable_name().to_string());
            Ok(())
        })
    }

    fn set_toggle_switch_style(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        _palette: (
            crate::Color,
            crate::Color,
            crate::Color,
            crate::Color,
            crate::Color,
        ),
    ) -> PlatformResult<()> {
        let control = self.with_control_ref(window_id, control_id)?;
        match &control.kind {
            ControlKind::ToggleSwitch { .. } => Ok(()),
            _ => Err(Self::unsupported_control_kind(
                "SetToggleSwitchStyle",
                "ToggleSwitch",
            )),
        }
    }

    fn set_scroll_position(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        vertical_pos: u32,
        horizontal_pos: u32,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| {
            control.scroll_vertical = vertical_pos;
            control.scroll_horizontal = horizontal_pos;
            Ok(())
        })
    }

    fn click_menu_action(
        &self,
        window_id: WindowId,
        action_id: MenuActionId,
    ) -> PlatformResult<()> {
        let window = self.window(window_id)?;
        if find_menu_action(&window.menu_items, action_id.raw()).is_none() {
            return Err(PlatformError::InvalidHandle(format!(
                "MenuActionId {} not found in window {window_id:?}",
                action_id.raw()
            )));
        }
        Ok(())
    }

    fn validate_visible_enabled_tree_view(
        &self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<()> {
        self.validate_visible_enabled_control(window_id, control_id, "tree action")?;
        let control = self.with_control_ref(window_id, control_id)?;
        match &control.kind {
            ControlKind::TreeView { .. } => Ok(()),
            _ => Err(Self::unsupported_control_kind("tree action", "TreeView")),
        }
    }

    fn validate_visible_enabled_scrollable(
        &self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<()> {
        self.validate_visible_enabled_control(window_id, control_id, "scroll")?;
        let control = self.with_control_ref(window_id, control_id)?;
        match &control.kind {
            ControlKind::Input { .. } | ControlKind::RichEdit { .. } => Ok(()),
            _ => Err(Self::unsupported_control_kind("scroll", "Input/RichEdit")),
        }
    }

    fn validate_window_visible(&self, window_id: WindowId) -> PlatformResult<()> {
        let window = self.window(window_id)?;
        if window.closed || !window.shown {
            return Err(PlatformError::InvalidHandle(format!(
                "WindowId {window_id:?} is not visible for menu action"
            )));
        }
        Ok(())
    }

    fn find_tree_item(
        &self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: TreeItemId,
    ) -> PlatformResult<&TreeItemNode> {
        let control = self.with_control_ref(window_id, control_id)?;
        match &control.kind {
            ControlKind::TreeView { items, .. } => {
                tree_items_find(items, item_id).ok_or_else(|| {
                    PlatformError::InvalidHandle(format!(
                        "TreeItemId {item_id:?} not found in window {window_id:?}"
                    ))
                })
            }
            _ => Err(Self::unsupported_control_kind("RedrawTreeItem", "TreeView")),
        }
    }

    fn toggle_tree_item(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: TreeItemId,
    ) -> PlatformResult<Option<CheckState>> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::TreeView { items, .. } => {
                let item = tree_items_find_mut(items, item_id).ok_or_else(|| {
                    PlatformError::InvalidHandle(format!(
                        "TreeItemId {item_id:?} not found in window {window_id:?}"
                    ))
                })?;
                if item.state == CheckState::Hidden {
                    return Ok(None);
                }
                item.state = match item.state {
                    CheckState::Checked => CheckState::Unchecked,
                    CheckState::Unchecked => CheckState::Checked,
                    CheckState::Hidden => CheckState::Hidden,
                };
                Ok(Some(item.state))
            }
            _ => Err(Self::unsupported_control_kind(
                "toggle_tree_item",
                "TreeView",
            )),
        })
    }

    fn set_rich_edit_content(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        rtf_text: String,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::RichEdit { text } => {
                *text = rtf_text;
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind(
                "SetRichEditContent",
                "RichEdit",
            )),
        })
    }

    fn update_label_text(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        text: String,
        severity: MessageSeverity,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::Label {
                text: current_text,
                severity: current_severity,
                ..
            } => {
                *current_text = text;
                *current_severity = severity;
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind("UpdateLabelText", "Label")),
        })
    }

    fn set_control_text(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        text: String,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::Button { text: current }
            | ControlKind::Label { text: current, .. }
            | ControlKind::Input { text: current, .. }
            | ControlKind::RichEdit { text: current }
            | ControlKind::CheckBox { text: current, .. }
            | ControlKind::RadioButton { text: current, .. }
            | ControlKind::ToggleSwitch { label: current, .. } => {
                *current = text;
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind(
                "SetControlText",
                "Button/Label/Input/RichEdit/CheckBox/RadioButton/ToggleSwitch",
            )),
        })
    }

    fn set_input_text(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        text: String,
    ) -> PlatformResult<()> {
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::Input { text: current, .. } => {
                *current = text;
                Ok(())
            }
            _ => Err(Self::unsupported_control_kind("SetInputText", "Input")),
        })
    }

    fn select_list_box_item(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: ListBoxItemId,
    ) -> PlatformResult<()> {
        self.validate_visible_enabled_control(window_id, control_id, "select_row")?;
        self.set_list_box_selection(window_id, control_id, item_id)
    }

    fn toggle_control(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<AppEvent> {
        self.validate_visible_enabled_control(window_id, control_id, "toggle")?;
        self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
            ControlKind::CheckBox { checked, .. } => {
                *checked = !*checked;
                Ok(AppEvent::CheckBoxToggled {
                    window_id,
                    control_id,
                    checked: *checked,
                })
            }
            ControlKind::ToggleSwitch { checked, .. } => {
                *checked = !*checked;
                Ok(AppEvent::ToggleSwitchToggled {
                    window_id,
                    control_id,
                    checked: *checked,
                })
            }
            _ => Err(Self::unsupported_control_kind(
                "toggle",
                "CheckBox/ToggleSwitch",
            )),
        })
    }

    fn select_radio_button(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<()> {
        self.validate_visible_enabled_control(window_id, control_id, "select_radio")?;
        let _parent =
            self.with_control_mut(window_id, control_id, |control| match &mut control.kind {
                ControlKind::RadioButton { checked, .. } => {
                    *checked = true;
                    Ok(control.parent_control_id)
                }
                _ => Err(Self::unsupported_control_kind(
                    "select_radio",
                    "RadioButton",
                )),
            })?;
        self.clear_other_radio_buttons(window_id, control_id)?;
        Ok(())
    }

    fn clear_other_radio_buttons(
        &mut self,
        window_id: WindowId,
        selected_control_id: ControlId,
    ) -> PlatformResult<()> {
        let window = self.window_mut(window_id)?;
        let selected_parent = window
            .controls
            .get(&selected_control_id.raw())
            .map(|control| control.parent_control_id)
            .ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "Control ID {} not found in window {window_id:?}",
                    selected_control_id.raw()
                ))
            })?;
        let mut radios = window
            .controls
            .values()
            .filter_map(|control| match &control.kind {
                ControlKind::RadioButton { group_start, .. } => Some((
                    control.creation_order,
                    control.control_id,
                    control.parent_control_id,
                    *group_start,
                )),
                _ => None,
            })
            .filter(|(_, _, parent_control_id, _)| *parent_control_id == selected_parent)
            .collect::<Vec<_>>();
        radios.sort_by_key(|(order, _, _, _)| *order);
        let selected_index = radios
            .iter()
            .position(|(_, control_id, _, _)| *control_id == selected_control_id)
            .ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "RadioButton {} not found in window {window_id:?}",
                    selected_control_id.raw()
                ))
            })?;

        let mut start_index = selected_index;
        while start_index > 0 && !radios[start_index].3 {
            start_index -= 1;
        }
        if !radios[start_index].3 && start_index != selected_index {
            start_index = selected_index;
        }

        let mut end_index = selected_index + 1;
        while end_index < radios.len() && !radios[end_index].3 {
            end_index += 1;
        }

        for (_, control_id, _, _) in radios[start_index..end_index].iter().copied() {
            if control_id == selected_control_id {
                continue;
            }
            if let Some(control) = window.controls.get_mut(&control_id.raw())
                && let ControlKind::RadioButton { checked, .. } = &mut control.kind
            {
                *checked = false;
            }
        }
        Ok(())
    }

    fn validate_visible_enabled_control(
        &self,
        window_id: WindowId,
        control_id: ControlId,
        action: &str,
    ) -> PlatformResult<()> {
        let window = self.window(window_id)?;
        if !window.shown || window.closed {
            return Err(PlatformError::InvalidHandle(format!(
                "Control ID {} in window {window_id:?} cannot be used for {action} because the window is not shown",
                control_id.raw()
            )));
        }
        let control = window.controls.get(&control_id.raw()).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "Control ID {} not found in window {window_id:?} for {action}",
                control_id.raw()
            ))
        })?;
        if !control.enabled {
            return Err(PlatformError::InvalidHandle(format!(
                "Control ID {} in window {window_id:?} is disabled for {action}",
                control_id.raw()
            )));
        }
        Ok(())
    }

    fn validate_visible_enabled_input(
        &self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<()> {
        self.validate_visible_enabled_control(window_id, control_id, "set_text")?;
        let control = self.with_control_ref(window_id, control_id)?;
        match &control.kind {
            ControlKind::Input { read_only, .. } => {
                if *read_only {
                    Err(PlatformError::InvalidHandle(format!(
                        "Control ID {} in window {window_id:?} is read-only for set_text",
                        control_id.raw()
                    )))
                } else {
                    Ok(())
                }
            }
            _ => Err(Self::unsupported_control_kind("set_text", "Input")),
        }
    }

    fn validate_visible_enabled_combo(
        &self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<()> {
        self.validate_visible_enabled_control(window_id, control_id, "select_combo")?;
        let control = self.with_control_ref(window_id, control_id)?;
        match &control.kind {
            ControlKind::ComboBox { .. } => Ok(()),
            _ => Err(Self::unsupported_control_kind("select_combo", "ComboBox")),
        }
    }

    fn validate_visible_enabled_tab(
        &self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<()> {
        self.validate_visible_enabled_control(window_id, control_id, "select_tab")?;
        let control = self.with_control_ref(window_id, control_id)?;
        match &control.kind {
            ControlKind::TabBar { .. } => Ok(()),
            _ => Err(Self::unsupported_control_kind("select_tab", "TabBar")),
        }
    }

    fn validate_visible_enabled_button(
        &self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<()> {
        self.validate_visible_enabled_control(window_id, control_id, "click")?;
        let control = self.with_control_ref(window_id, control_id)?;
        match &control.kind {
            ControlKind::Button { .. } => Ok(()),
            _ => Err(Self::unsupported_control_kind("click", "Button")),
        }
    }

    fn create_control(
        &mut self,
        window_id: WindowId,
        parent_control_id: Option<ControlId>,
        control_id: ControlId,
        kind: ControlKind,
    ) -> PlatformResult<()> {
        self.with_window_mut(window_id, |window| {
            window.ensure_not_closed()?;
            if window.controls.contains_key(&control_id.raw()) {
                return Err(PlatformError::OperationFailed(format!(
                    "Control with ID {} already exists for window {window_id:?}",
                    control_id.raw()
                )));
            }
            if let Some(parent_id) = parent_control_id
                && !window.controls.contains_key(&parent_id.raw())
            {
                return Err(PlatformError::InvalidHandle(format!(
                    "Parent control with ID {} not found for window {window_id:?}",
                    parent_id.raw()
                )));
            }
            window.controls.insert(
                control_id.raw(),
                ControlState {
                    control_id,
                    parent_control_id,
                    creation_order: window.next_control_order,
                    enabled: true,
                    selected_all: false,
                    scroll_vertical: 0,
                    scroll_horizontal: 0,
                    style_id: None,
                    kind,
                },
            );
            window.next_control_order += 1;
            Ok(())
        })
    }

    fn with_window_mut<T>(
        &mut self,
        window_id: WindowId,
        f: impl FnOnce(&mut WindowState) -> PlatformResult<T>,
    ) -> PlatformResult<T> {
        let window = self.windows.get_mut(&window_id.raw()).ok_or_else(|| {
            PlatformError::InvalidHandle(format!("WindowId {window_id:?} not found"))
        })?;
        f(window)
    }

    fn with_window_and_control_mut<T>(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        f: impl FnOnce(&mut WindowState, &mut ControlState) -> PlatformResult<T>,
    ) -> PlatformResult<T> {
        let window = self.windows.get_mut(&window_id.raw()).ok_or_else(|| {
            PlatformError::InvalidHandle(format!("WindowId {window_id:?} not found"))
        })?;
        let mut control = window.controls.remove(&control_id.raw()).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "Control ID {} not found in window {window_id:?}",
                control_id.raw()
            ))
        })?;
        let result = f(window, &mut control);
        window.controls.insert(control_id.raw(), control);
        result
    }

    fn with_control_mut<T>(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        f: impl FnOnce(&mut ControlState) -> PlatformResult<T>,
    ) -> PlatformResult<T> {
        self.with_window_mut(window_id, |window| {
            let control = window.controls.get_mut(&control_id.raw()).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "Control ID {} not found in window {window_id:?}",
                    control_id.raw()
                ))
            })?;
            f(control)
        })
    }

    fn with_control_ref(
        &self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<&ControlState> {
        let window = self.window(window_id)?;
        window.controls.get(&control_id.raw()).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "Control ID {} not found in window {window_id:?}",
                control_id.raw()
            ))
        })
    }

    fn ensure_window_exists(&self, window_id: WindowId) -> PlatformResult<()> {
        self.window(window_id).map(|_| ())
    }

    fn window(&self, window_id: WindowId) -> PlatformResult<&WindowState> {
        self.windows.get(&window_id.raw()).ok_or_else(|| {
            PlatformError::InvalidHandle(format!("WindowId {window_id:?} not found"))
        })
    }

    fn window_mut(&mut self, window_id: WindowId) -> PlatformResult<&mut WindowState> {
        self.windows.get_mut(&window_id.raw()).ok_or_else(|| {
            PlatformError::InvalidHandle(format!("WindowId {window_id:?} not found"))
        })
    }
}

#[derive(Debug)]
struct WindowState {
    window_id: WindowId,
    title: String,
    width: i32,
    height: i32,
    shown: bool,
    closed: bool,
    focused_control_id: Option<ControlId>,
    menu_items: Vec<MenuNode>,
    layout_rules: Vec<LayoutRule>,
    controls: BTreeMap<i32, ControlState>,
    next_control_order: usize,
}

impl WindowState {
    fn new(window_id: WindowId, title: &str, width: i32, height: i32) -> Self {
        Self {
            window_id,
            title: title.to_string(),
            width,
            height,
            shown: false,
            closed: false,
            focused_control_id: None,
            menu_items: Vec::new(),
            layout_rules: Vec::new(),
            controls: BTreeMap::new(),
            next_control_order: 0,
        }
    }

    fn ensure_open(&self) -> PlatformResult<()> {
        if self.closed {
            return Err(PlatformError::InvalidHandle(format!(
                "WindowId {:?} is closed",
                self.window_id
            )));
        }
        Ok(())
    }

    fn ensure_not_closed(&self) -> PlatformResult<()> {
        if self.closed {
            return Err(PlatformError::InvalidHandle(format!(
                "WindowId {:?} is closed",
                self.window_id
            )));
        }
        Ok(())
    }

    fn snapshot(&self) -> WindowSnapshot {
        WindowSnapshot {
            id: self.window_id.raw(),
            title: self.title.clone(),
            width: self.width,
            height: self.height,
            shown: self.shown,
            closed: self.closed,
            focused_control_id: self.focused_control_id.map(|id| id.raw()),
            menu: self.menu_items.iter().map(MenuNodeSnapshot::from).collect(),
            layout_rules: self
                .layout_rules
                .iter()
                .map(LayoutRuleSnapshot::from)
                .collect(),
            controls: self.controls.values().map(ControlSnapshot::from).collect(),
        }
    }
}

#[derive(Debug)]
struct ControlState {
    control_id: ControlId,
    parent_control_id: Option<ControlId>,
    creation_order: usize,
    enabled: bool,
    selected_all: bool,
    // Reserved for Phase 2 surface area; Phase 1 only records logical state and focus.
    scroll_vertical: u32,
    // Reserved for Phase 2 surface area; Phase 1 only records logical state and focus.
    scroll_horizontal: u32,
    // Reserved for Phase 2 styling/application commands.
    style_id: Option<String>,
    kind: ControlKind,
}

#[derive(Debug)]
enum ControlKind {
    Panel,
    Button {
        text: String,
    },
    Label {
        text: String,
        class: LabelClass,
        severity: MessageSeverity,
    },
    Input {
        text: String,
        read_only: bool,
        multiline: bool,
        vertical_scroll: bool,
    },
    RichEdit {
        text: String,
    },
    ListBox {
        items: Vec<ListBoxItemDescriptor>,
        selected_item_id: Option<ListBoxItemId>,
        badge_column_width: u16,
        density: ListBoxRowDensity,
    },
    ComboBox {
        items: Vec<String>,
        selected_index: Option<usize>,
    },
    RadioButton {
        text: String,
        checked: bool,
        group_start: bool,
    },
    CheckBox {
        text: String,
        checked: bool,
    },
    TabBar {
        items: Vec<String>,
        selected_index: Option<usize>,
    },
    TreeView {
        items: Vec<TreeItemNode>,
        selected_item_id: Option<TreeItemId>,
    },
    Chart {
        data: Option<ChartDataState>,
    },
    ToggleSwitch {
        label: String,
        checked: bool,
    },
    ProgressBar {
        min: u32,
        max: u32,
        position: u32,
    },
    Splitter {
        orientation: SplitterOrientation,
    },
}

#[derive(Debug)]
struct TreeItemNode {
    id: TreeItemId,
    text: String,
    is_folder: bool,
    state: CheckState,
    expanded: bool,
    style_override: Option<String>,
    children: Vec<TreeItemNode>,
}

#[derive(Debug)]
struct ChartDataState {
    lines: Vec<ChartLineState>,
    week_labels: Vec<String>,
    is_loading: bool,
    show_x_axis_labels: bool,
    show_y_axis_labels: bool,
    show_end_labels: bool,
}

#[derive(Debug)]
struct ChartLineState {
    label: String,
    weekly_counts: Vec<u32>,
    end_label: Option<String>,
    emphasis: String,
}

#[derive(Debug)]
struct MenuNode {
    action: Option<u32>,
    text: String,
    children: Vec<MenuNode>,
}

impl TreeItemNode {
    fn from_descriptor(descriptor: &TreeItemDescriptor) -> Self {
        Self {
            id: descriptor.id,
            text: descriptor.text.clone(),
            is_folder: descriptor.is_folder,
            state: descriptor.state,
            expanded: false,
            style_override: descriptor
                .style_override
                .map(|style_id| style_id.stable_name().to_string()),
            children: descriptor
                .children
                .iter()
                .map(TreeItemNode::from_descriptor)
                .collect(),
        }
    }

    fn expand_recursive(&mut self) {
        self.expanded = true;
        for child in &mut self.children {
            child.expand_recursive();
        }
    }
}

impl ChartDataState {
    fn from_packet(packet: ChartDataPacket) -> Self {
        Self {
            lines: packet
                .lines
                .into_iter()
                .map(ChartLineState::from_line)
                .collect(),
            week_labels: packet.week_labels,
            is_loading: packet.is_loading,
            show_x_axis_labels: packet.show_x_axis_labels,
            show_y_axis_labels: packet.show_y_axis_labels,
            show_end_labels: packet.show_end_labels,
        }
    }
}

impl ChartLineState {
    fn from_line(line: crate::ChartLineData) -> Self {
        Self {
            label: line.label,
            weekly_counts: line.weekly_counts,
            end_label: line.end_label,
            emphasis: line.emphasis.stable_name().to_string(),
        }
    }
}

impl MenuNode {
    fn from_config(config: &crate::MenuItemConfig) -> Self {
        Self {
            action: config.action.map(|action| action.raw()),
            text: config.text.clone(),
            children: config.children.iter().map(MenuNode::from_config).collect(),
        }
    }
}

fn tree_items_find(items: &[TreeItemNode], item_id: TreeItemId) -> Option<&TreeItemNode> {
    for item in items {
        if item.id == item_id {
            return Some(item);
        }
        if let Some(found) = tree_items_find(&item.children, item_id) {
            return Some(found);
        }
    }
    None
}

fn tree_items_find_mut(
    items: &mut [TreeItemNode],
    item_id: TreeItemId,
) -> Option<&mut TreeItemNode> {
    for item in items {
        if item.id == item_id {
            return Some(item);
        }
        if let Some(found) = tree_items_find_mut(&mut item.children, item_id) {
            return Some(found);
        }
    }
    None
}

fn tree_items_contain_id(items: &[TreeItemNode], item_id: TreeItemId) -> bool {
    tree_items_find(items, item_id).is_some()
}

fn find_menu_action(items: &[MenuNode], action_id: u32) -> Option<&MenuNode> {
    for item in items {
        if item.action == Some(action_id) {
            return Some(item);
        }
        if let Some(found) = find_menu_action(&item.children, action_id) {
            return Some(found);
        }
    }
    None
}

#[derive(Debug, Serialize)]
struct HeadlessSnapshot {
    app_name: String,
    quitting: bool,
    markers: Vec<String>,
    dialog_requests: Vec<DialogRequestSnapshot>,
    windows: Vec<WindowSnapshot>,
}

#[derive(Debug, Serialize)]
struct ProtocolSnapshot {
    app_name: String,
    dialog_requests: Vec<DialogRequestSnapshot>,
    windows: Vec<WindowSnapshot>,
}

#[derive(Debug, Serialize)]
struct WindowSnapshot {
    id: usize,
    title: String,
    width: i32,
    height: i32,
    shown: bool,
    closed: bool,
    focused_control_id: Option<i32>,
    menu: Vec<MenuNodeSnapshot>,
    layout_rules: Vec<LayoutRuleSnapshot>,
    controls: Vec<ControlSnapshot>,
}

#[derive(Debug, Serialize)]
struct LayoutRuleSnapshot {
    control_id: i32,
    parent_control_id: Option<i32>,
    dock_style: String,
    order: u32,
    fixed_size: Option<i32>,
    margin: [i32; 4],
}

impl From<&LayoutRule> for LayoutRuleSnapshot {
    fn from(rule: &LayoutRule) -> Self {
        Self {
            control_id: rule.control_id.raw(),
            parent_control_id: rule.parent_control_id.map(|id| id.raw()),
            dock_style: rule.dock_style.stable_name().to_string(),
            order: rule.order,
            fixed_size: rule.fixed_size,
            margin: [rule.margin.0, rule.margin.1, rule.margin.2, rule.margin.3],
        }
    }
}

#[derive(Debug, Serialize)]
struct MenuNodeSnapshot {
    action: Option<u32>,
    text: String,
    children: Vec<MenuNodeSnapshot>,
}

impl From<&MenuNode> for MenuNodeSnapshot {
    fn from(node: &MenuNode) -> Self {
        Self {
            action: node.action,
            text: node.text.clone(),
            children: node.children.iter().map(MenuNodeSnapshot::from).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ControlSnapshot {
    Panel {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
    },
    Button {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        text: String,
    },
    Label {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        text: String,
        class: String,
        severity: String,
    },
    Input {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        text: String,
        read_only: bool,
        multiline: bool,
        vertical_scroll: bool,
    },
    RichEdit {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        text: String,
    },
    ListBox {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        items: Vec<ListBoxItemSnapshot>,
        selected_item_id: Option<u64>,
        badge_column_width: u16,
        density: String,
    },
    ComboBox {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        items: Vec<String>,
        selected_index: Option<usize>,
    },
    RadioButton {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        text: String,
        checked: bool,
        group_start: bool,
    },
    CheckBox {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        text: String,
        checked: bool,
    },
    TabBar {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        items: Vec<String>,
        selected_index: Option<usize>,
    },
    TreeView {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        items: Vec<TreeItemSnapshot>,
        selected_item_id: Option<u64>,
    },
    Chart {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        data: Option<ChartDataSnapshot>,
    },
    ToggleSwitch {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        label: String,
        checked: bool,
    },
    ProgressBar {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        min: u32,
        max: u32,
        position: u32,
    },
    Splitter {
        id: i32,
        parent_control_id: Option<i32>,
        enabled: bool,
        selected_all: bool,
        scroll_vertical: u32,
        scroll_horizontal: u32,
        style_id: Option<String>,
        orientation: String,
    },
}

#[derive(Debug, Serialize)]
struct TreeItemSnapshot {
    id: u64,
    text: String,
    is_folder: bool,
    state: String,
    expanded: bool,
    style_override: Option<String>,
    children: Vec<TreeItemSnapshot>,
}

impl From<&TreeItemNode> for TreeItemSnapshot {
    fn from(node: &TreeItemNode) -> Self {
        Self {
            id: node.id.raw(),
            text: node.text.clone(),
            is_folder: node.is_folder,
            state: node.state.stable_name().to_string(),
            expanded: node.expanded,
            style_override: node.style_override.clone(),
            children: node.children.iter().map(TreeItemSnapshot::from).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ChartDataSnapshot {
    lines: Vec<ChartLineSnapshot>,
    week_labels: Vec<String>,
    is_loading: bool,
    show_x_axis_labels: bool,
    show_y_axis_labels: bool,
    show_end_labels: bool,
}

impl From<&ChartDataState> for ChartDataSnapshot {
    fn from(data: &ChartDataState) -> Self {
        Self {
            lines: data.lines.iter().map(ChartLineSnapshot::from).collect(),
            week_labels: data.week_labels.clone(),
            is_loading: data.is_loading,
            show_x_axis_labels: data.show_x_axis_labels,
            show_y_axis_labels: data.show_y_axis_labels,
            show_end_labels: data.show_end_labels,
        }
    }
}

#[derive(Debug, Serialize)]
struct ChartLineSnapshot {
    label: String,
    weekly_counts: Vec<u32>,
    end_label: Option<String>,
    emphasis: String,
}

impl From<&ChartLineState> for ChartLineSnapshot {
    fn from(line: &ChartLineState) -> Self {
        Self {
            label: line.label.clone(),
            weekly_counts: line.weekly_counts.clone(),
            end_label: line.end_label.clone(),
            emphasis: line.emphasis.clone(),
        }
    }
}

impl From<&ControlState> for ControlSnapshot {
    fn from(control: &ControlState) -> Self {
        let common = (
            control.control_id.raw(),
            control.parent_control_id.map(|id| id.raw()),
            control.enabled,
            control.selected_all,
            control.scroll_vertical,
            control.scroll_horizontal,
            control.style_id.clone(),
        );
        match &control.kind {
            ControlKind::Panel => Self::Panel {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
            },
            ControlKind::Button { text } => Self::Button {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                text: text.clone(),
            },
            ControlKind::Label {
                text,
                class,
                severity,
            } => Self::Label {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                text: text.clone(),
                class: class.stable_name().to_string(),
                severity: severity.stable_name().to_string(),
            },
            ControlKind::Input {
                text,
                read_only,
                multiline,
                vertical_scroll,
            } => Self::Input {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                text: text.clone(),
                read_only: *read_only,
                multiline: *multiline,
                vertical_scroll: *vertical_scroll,
            },
            ControlKind::RichEdit { text } => Self::RichEdit {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                text: text.clone(),
            },
            ControlKind::ListBox {
                items,
                selected_item_id,
                badge_column_width,
                density,
            } => Self::ListBox {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                items: items.iter().map(ListBoxItemSnapshot::from).collect(),
                selected_item_id: selected_item_id.map(|id| id.raw()),
                badge_column_width: *badge_column_width,
                density: density.stable_name().to_string(),
            },
            ControlKind::ComboBox {
                items,
                selected_index,
            } => Self::ComboBox {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                items: items.clone(),
                selected_index: *selected_index,
            },
            ControlKind::RadioButton {
                text,
                checked,
                group_start,
            } => Self::RadioButton {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                text: text.clone(),
                checked: *checked,
                group_start: *group_start,
            },
            ControlKind::CheckBox { text, checked } => Self::CheckBox {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                text: text.clone(),
                checked: *checked,
            },
            ControlKind::TabBar {
                items,
                selected_index,
            } => Self::TabBar {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                items: items.clone(),
                selected_index: *selected_index,
            },
            ControlKind::TreeView {
                items,
                selected_item_id,
            } => Self::TreeView {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                items: items.iter().map(TreeItemSnapshot::from).collect(),
                selected_item_id: selected_item_id.map(|id| id.raw()),
            },
            ControlKind::Chart { data } => Self::Chart {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                data: data.as_ref().map(ChartDataSnapshot::from),
            },
            ControlKind::ToggleSwitch { label, checked } => Self::ToggleSwitch {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                label: label.clone(),
                checked: *checked,
            },
            ControlKind::ProgressBar { min, max, position } => Self::ProgressBar {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                min: *min,
                max: *max,
                position: *position,
            },
            ControlKind::Splitter { orientation } => Self::Splitter {
                id: common.0,
                parent_control_id: common.1,
                enabled: common.2,
                selected_all: common.3,
                scroll_vertical: common.4,
                scroll_horizontal: common.5,
                style_id: common.6,
                orientation: orientation.stable_name().to_string(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
struct DialogRequestSnapshot {
    kind: String,
    window_id: usize,
    title: String,
    prompt: Option<String>,
    context_tag: Option<String>,
    details: DialogRequestDetailsSnapshot,
}

impl From<&DialogRequest> for DialogRequestSnapshot {
    fn from(request: &DialogRequest) -> Self {
        Self {
            kind: request.kind.stable_name().to_string(),
            window_id: request.window_id.raw(),
            title: request.title.clone(),
            prompt: request.prompt.clone(),
            context_tag: request.context_tag.clone(),
            details: DialogRequestDetailsSnapshot::from(&request.details),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DialogRequestDetailsSnapshot {
    SaveFile {
        default_filename: String,
        filter_spec: String,
        initial_dir: Option<PathBuf>,
    },
    OpenFile {
        filter_spec: String,
        initial_dir: Option<PathBuf>,
    },
    ProfileSelection {
        available_profiles: Vec<String>,
    },
    Input {
        default_text: Option<String>,
    },
    ExcludePatterns {
        patterns: String,
    },
    Form {
        form: FormDialogSnapshot,
    },
    MessageBox {
        message: String,
        severity: String,
    },
    FolderPicker {
        initial_dir: Option<PathBuf>,
    },
}

impl From<&DialogRequestDetails> for DialogRequestDetailsSnapshot {
    fn from(details: &DialogRequestDetails) -> Self {
        match details {
            DialogRequestDetails::SaveFile {
                default_filename,
                filter_spec,
                initial_dir,
            } => Self::SaveFile {
                default_filename: default_filename.clone(),
                filter_spec: filter_spec.clone(),
                initial_dir: initial_dir.clone(),
            },
            DialogRequestDetails::OpenFile {
                filter_spec,
                initial_dir,
            } => Self::OpenFile {
                filter_spec: filter_spec.clone(),
                initial_dir: initial_dir.clone(),
            },
            DialogRequestDetails::ProfileSelection { available_profiles } => {
                Self::ProfileSelection {
                    available_profiles: available_profiles.clone(),
                }
            }
            DialogRequestDetails::Input { default_text } => Self::Input {
                default_text: default_text.clone(),
            },
            DialogRequestDetails::ExcludePatterns { patterns } => Self::ExcludePatterns {
                patterns: patterns.clone(),
            },
            DialogRequestDetails::Form { form } => Self::Form {
                form: FormDialogSnapshot::from(form),
            },
            DialogRequestDetails::MessageBox { message, severity } => Self::MessageBox {
                message: message.clone(),
                severity: severity.stable_name().to_string(),
            },
            DialogRequestDetails::FolderPicker { initial_dir } => Self::FolderPicker {
                initial_dir: initial_dir.clone(),
            },
        }
    }
}

impl ProtocolDialogScriptEntry {
    fn into_runtime(self) -> PlatformResult<DialogScriptEntry> {
        let matcher = self.matcher.into_runtime()?;
        let outcome = self.outcome.into_runtime()?;
        if matcher.kind != outcome.kind() {
            return Err(PlatformError::OperationFailed(format!(
                "Dialog responder matcher kind '{}' does not match outcome kind '{}'",
                matcher.kind.stable_name(),
                outcome.kind().stable_name()
            )));
        }
        Ok(DialogScriptEntry { matcher, outcome })
    }
}

impl ProtocolDialogMatcher {
    fn into_runtime(self) -> PlatformResult<DialogMatcher> {
        let kind = DialogKind::from_stable_name(&self.kind).ok_or_else(|| {
            PlatformError::OperationFailed(format!(
                "Unknown dialog responder matcher kind '{}'",
                self.kind
            ))
        })?;
        Ok(DialogMatcher {
            kind,
            window_id: self.window_id.map(WindowId::new),
            title: self.title,
            prompt: self.prompt,
            context_tag: self.context_tag,
        })
    }
}

impl ProtocolDialogOutcome {
    fn into_runtime(self) -> PlatformResult<DialogOutcome> {
        Ok(match self {
            Self::SaveFile { path } => DialogOutcome::SaveFile {
                result: path.map(PathBuf::from),
            },
            Self::OpenFile { path } => DialogOutcome::OpenFile {
                result: path.map(PathBuf::from),
            },
            Self::ProfileSelection {
                chosen_profile_name,
                create_new_requested,
                user_cancelled,
            } => DialogOutcome::ProfileSelection {
                chosen_profile_name,
                create_new_requested,
                user_cancelled,
            },
            Self::Input { text } => DialogOutcome::Input { text },
            Self::ExcludePatterns { saved, patterns } => {
                DialogOutcome::ExcludePatterns { saved, patterns }
            }
            Self::Form {
                confirmed,
                field_values,
            } => DialogOutcome::Form {
                confirmed,
                field_values: field_values
                    .into_iter()
                    .map(ProtocolFormFieldValue::into_runtime)
                    .collect::<PlatformResult<Vec<_>>>()?,
            },
            Self::FolderPicker { path } => DialogOutcome::FolderPicker {
                path: path.map(PathBuf::from),
            },
            Self::MessageBox => DialogOutcome::MessageBox,
        })
    }
}

impl ProtocolFormFieldValue {
    fn into_runtime(self) -> PlatformResult<crate::FormFieldValue> {
        Ok(match self {
            Self::Text { field_id, value } => crate::FormFieldValue::Text { field_id, value },
            Self::CheckBox { field_id, checked } => {
                crate::FormFieldValue::CheckBox { field_id, checked }
            }
        })
    }
}

#[derive(Debug, Serialize)]
struct FormDialogSnapshot {
    title: String,
    context_tag: String,
    rows: Vec<FormRowSnapshot>,
    fields: Vec<FormFieldSnapshot>,
    buttons: FormButtonsSnapshot,
}

impl From<&crate::FormDialogDescriptor> for FormDialogSnapshot {
    fn from(form: &crate::FormDialogDescriptor) -> Self {
        Self {
            title: form.title.clone(),
            context_tag: form.context_tag.clone(),
            rows: form.rows.iter().map(FormRowSnapshot::from).collect(),
            fields: form.fields.iter().map(FormFieldSnapshot::from).collect(),
            buttons: FormButtonsSnapshot::from(&form.buttons),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum FormRowSnapshot {
    ReadOnlyText { label: String, value: String },
    Note { text: String, severity: String },
}

impl From<&crate::FormRow> for FormRowSnapshot {
    fn from(row: &crate::FormRow) -> Self {
        match row {
            crate::FormRow::ReadOnlyText { label, value } => Self::ReadOnlyText {
                label: label.clone(),
                value: value.clone(),
            },
            crate::FormRow::Note { text, severity } => Self::Note {
                text: text.clone(),
                severity: severity.stable_name().to_string(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum FormFieldSnapshot {
    TextInput {
        field_id: String,
        label: String,
        value: String,
        validation: String,
        live_warning: Option<FormFileExistsWarningSnapshot>,
    },
    CheckBox {
        field_id: String,
        label: String,
        checked: bool,
    },
}

impl From<&crate::FormField> for FormFieldSnapshot {
    fn from(field: &crate::FormField) -> Self {
        match field {
            crate::FormField::TextInput {
                field_id,
                label,
                value,
                validation,
                live_warning,
            } => Self::TextInput {
                field_id: field_id.clone(),
                label: label.clone(),
                value: value.clone(),
                validation: validation.stable_name().to_string(),
                live_warning: live_warning
                    .as_ref()
                    .map(FormFileExistsWarningSnapshot::from),
            },
            crate::FormField::CheckBox {
                field_id,
                label,
                checked,
            } => Self::CheckBox {
                field_id: field_id.clone(),
                label: label.clone(),
                checked: *checked,
            },
        }
    }
}

#[derive(Debug, Serialize)]
struct FormFileExistsWarningSnapshot {
    base_dir: PathBuf,
    message: String,
}

impl From<&crate::FormFileExistsWarning> for FormFileExistsWarningSnapshot {
    fn from(warning: &crate::FormFileExistsWarning) -> Self {
        Self {
            base_dir: warning.base_dir.clone(),
            message: warning.message.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct FormButtonsSnapshot {
    confirm_label: String,
    cancel_label: String,
    confirm_enabled: bool,
}

impl From<&crate::FormButtons> for FormButtonsSnapshot {
    fn from(buttons: &crate::FormButtons) -> Self {
        Self {
            confirm_label: buttons.confirm_label.clone(),
            cancel_label: buttons.cancel_label.clone(),
            confirm_enabled: buttons.confirm_enabled,
        }
    }
}

#[derive(Debug, Serialize)]
struct ListBoxItemSnapshot {
    id: u64,
    title: String,
    metadata: String,
    enabled: bool,
    badges: Vec<BadgeSnapshot>,
}

impl From<&ListBoxItemDescriptor> for ListBoxItemSnapshot {
    fn from(item: &ListBoxItemDescriptor) -> Self {
        Self {
            id: item.id.raw(),
            title: item.title.clone(),
            metadata: item.metadata.clone(),
            enabled: item.enabled,
            badges: item.badges.iter().map(BadgeSnapshot::from).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
struct BadgeSnapshot {
    text: String,
    style: String,
}

impl From<&BadgeDescriptor> for BadgeSnapshot {
    fn from(badge: &BadgeDescriptor) -> Self {
        Self {
            text: badge.text.clone(),
            style: badge.style.stable_name().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TreeItemId;
    use crate::{ChartLineData, ChartLineEmphasis, Color, MenuActionId, MenuItemConfig};
    use crate::{
        FormButtons, FormDialogDescriptor, FormField, FormFieldValue, FormRow, FormTextValidation,
    };
    use serde_json::Value;
    use std::io::{Cursor, Write};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    const BTN_CLICK_ME: ControlId = ControlId::new(101);

    struct TestHandler {
        events: Vec<AppEvent>,
        commands: VecDeque<PlatformCommand>,
    }

    impl PlatformEventHandler for TestHandler {
        fn handle_event(&mut self, event: AppEvent) {
            self.events.push(event);
        }

        fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
            self.commands.pop_front()
        }
    }

    struct SilentProvider;

    impl UiStateProvider for SilentProvider {
        fn is_tree_item_new(&self, _window_id: WindowId, _item_id: TreeItemId) -> bool {
            false
        }
    }

    struct RecordingWriter {
        bytes: Vec<u8>,
        flushes: usize,
    }

    impl RecordingWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                flushes: 0,
            }
        }

        fn into_string(self) -> String {
            String::from_utf8(self.bytes).unwrap()
        }
    }

    impl Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    fn parse_protocol_lines(output: &str) -> Vec<Value> {
        output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn started_harness(
        initial_commands: Vec<PlatformCommand>,
    ) -> (
        HeadlessHarness,
        WindowId,
        Arc<Mutex<TestHandler>>,
        Arc<Mutex<SilentProvider>>,
    ) {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(handler.clone(), provider.clone(), initial_commands)
            .unwrap();
        (harness, window_id, handler, provider)
    }

    #[test]
    fn protocol_hello_is_first_and_flushes_output_groups() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler,
                provider,
                vec![PlatformCommand::ShowWindow { window_id }],
            )
            .unwrap();

        let input = Cursor::new(Vec::<u8>::new());
        let mut writer = RecordingWriter::new();
        harness.run_protocol(input, &mut writer).unwrap();

        let flushes = writer.flushes;
        let output = writer.into_string();
        let lines = parse_protocol_lines(&output);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["type"], "hello");
        assert_eq!(lines[0]["protocol_version"], HEADLESS_PROTOCOL_VERSION);
        assert_eq!(lines[1]["type"], "bye");
        assert_eq!(flushes, 2);
    }

    #[test]
    fn protocol_actions_round_trip_and_wait_for_is_cursor_relative() {
        struct ProtocolHandler {
            events: Vec<AppEvent>,
            commands: VecDeque<PlatformCommand>,
            target_window: WindowId,
        }

        impl PlatformEventHandler for ProtocolHandler {
            fn handle_event(&mut self, event: AppEvent) {
                if matches!(
                    event,
                    AppEvent::ButtonClicked {
                        control_id,
                        ..
                    } if control_id == BTN_CLICK_ME
                ) {
                    self.commands.push_back(PlatformCommand::SetWindowTitle {
                        window_id: self.target_window,
                        title: "Clicked".into(),
                    });
                    self.commands.push_back(PlatformCommand::Checkpoint {
                        label: "done".into(),
                    });
                }
                self.events.push(event);
            }

            fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
                self.commands.pop_front()
            }
        }

        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(ProtocolHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
            target_window: window_id,
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler.clone(),
                provider,
                vec![
                    PlatformCommand::CreateButton {
                        window_id,
                        parent_control_id: None,
                        control_id: BTN_CLICK_ME,
                        text: "Click".into(),
                    },
                    PlatformCommand::ShowWindow { window_id },
                ],
            )
            .unwrap();

        let protocol_input = format!(
            "{}\n{}\n{}\n{}\n",
            serde_json::json!({
                "type": "snapshot",
                "request_id": 1
            }),
            serde_json::json!({
                "type": "action",
                "request_id": 2,
                "action": "click",
                "window_id": window_id.raw(),
                "control_id": BTN_CLICK_ME.raw()
            }),
            serde_json::json!({
                "type": "snapshot",
                "request_id": 3
            }),
            serde_json::json!({
                "type": "wait_for",
                "request_id": 4,
                "label": "done",
                "timeout_ms": 200
            })
        );

        let handler_for_thread = handler.clone();
        let delayed_marker =
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                handler_for_thread.lock().unwrap().commands.push_back(
                    PlatformCommand::Checkpoint {
                        label: "done".into(),
                    },
                );
            });

        let mut writer = RecordingWriter::new();
        harness
            .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
            .unwrap();
        delayed_marker.join().unwrap();

        let flushes = writer.flushes;
        let output = writer.into_string();
        let lines = parse_protocol_lines(&output);
        assert_eq!(
            lines
                .iter()
                .map(|line| line["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "hello", "snapshot", "marker", "ok", "snapshot", "marker", "ok", "bye",
            ]
        );
        assert_eq!(lines[1]["request_id"], 1);
        assert!(lines[1]["model"].get("markers").is_none());
        assert!(lines[1]["model"].get("quitting").is_none());
        assert_eq!(lines[3]["request_id"], 2);
        assert_eq!(lines[4]["model"]["windows"][0]["title"], "Clicked");
        assert!(lines[4]["model"].get("markers").is_none());
        assert!(lines[4]["model"].get("quitting").is_none());
        assert_eq!(lines[5]["label"], "done");
        assert_eq!(lines[6]["request_id"], 4);
        assert_eq!(flushes, 6);
    }

    #[test]
    fn protocol_recovers_request_ids_for_malformed_input() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler,
                provider,
                vec![PlatformCommand::ShowWindow { window_id }],
            )
            .unwrap();

        let protocol_input = concat!(
            "{",
            "\"type\":\"action\",",
            "\"request_id\":42,",
            "\"action\":\"click\",",
            "\"window_id\":1",
            "}\n",
            "{\"request_id\":99}\n",
            "{\"type\":\"snapshot\",\"request_id\":7}\n",
            "{not-json}\n",
        );

        let mut writer = RecordingWriter::new();
        harness
            .run_protocol(Cursor::new(protocol_input.as_bytes().to_vec()), &mut writer)
            .unwrap();

        let lines = parse_protocol_lines(&writer.into_string());
        assert_eq!(lines[0]["type"], "hello");
        assert_eq!(lines[1]["type"], "error");
        assert_eq!(lines[1]["request_id"], 42);
        assert!(lines[1]["message"].as_str().unwrap().contains("control_id"));
        assert_eq!(lines[2]["type"], "error");
        assert_eq!(lines[2]["request_id"], 99);
        assert!(lines[2]["message"].as_str().unwrap().contains("type"));
        assert_eq!(lines[3]["type"], "snapshot");
        assert_eq!(lines[3]["request_id"], 7);
        assert_eq!(lines[4]["type"], "error");
        assert!(lines[4]["request_id"].is_null());
        assert_eq!(lines[5]["type"], "bye");
    }

    #[test]
    fn protocol_dialogs_default_to_cancel_without_driver_scripting() {
        struct DialogHandler {
            events: Vec<AppEvent>,
            commands: VecDeque<PlatformCommand>,
            target_window: WindowId,
        }

        impl PlatformEventHandler for DialogHandler {
            fn handle_event(&mut self, event: AppEvent) {
                if matches!(event, AppEvent::ButtonClicked { .. }) {
                    self.commands
                        .push_back(PlatformCommand::ShowSaveFileDialog {
                            window_id: self.target_window,
                            title: "Save".into(),
                            default_filename: "export.txt".into(),
                            filter_spec: "*.txt".into(),
                            initial_dir: None,
                        });
                }
                self.events.push(event);
            }

            fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
                self.commands.pop_front()
            }
        }

        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(DialogHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
            target_window: window_id,
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler.clone(),
                provider,
                vec![
                    PlatformCommand::CreateButton {
                        window_id,
                        parent_control_id: None,
                        control_id: BTN_CLICK_ME,
                        text: "Save".into(),
                    },
                    PlatformCommand::ShowWindow { window_id },
                ],
            )
            .unwrap();

        let protocol_input = serde_json::json!({
            "type": "action",
            "request_id": 1,
            "action": "click",
            "window_id": window_id.raw(),
            "control_id": BTN_CLICK_ME.raw()
        })
        .to_string()
            + "\n";

        let mut writer = RecordingWriter::new();
        harness
            .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
            .unwrap();

        let events = &handler.lock().unwrap().events;
        assert!(events.iter().any(|event| matches!(
            event,
            AppEvent::FileSaveDialogCompleted { result: None, .. }
        )));
        let lines = parse_protocol_lines(&writer.into_string());
        assert_eq!(lines[0]["type"], "hello");
        assert_eq!(lines[1]["type"], "ok");
        assert_eq!(lines[2]["type"], "bye");
    }

    #[test]
    fn protocol_set_dialog_responder_scripts_form_dialog_and_round_trips_field_values() {
        struct DialogHandler {
            events: Vec<AppEvent>,
            commands: VecDeque<PlatformCommand>,
            target_window: WindowId,
        }

        impl PlatformEventHandler for DialogHandler {
            fn handle_event(&mut self, event: AppEvent) {
                if matches!(event, AppEvent::ButtonClicked { .. }) {
                    self.commands.push_back(PlatformCommand::ShowFormDialog {
                        window_id: self.target_window,
                        form: FormDialogDescriptor {
                            title: "Form".into(),
                            context_tag: "form-tag".into(),
                            rows: vec![],
                            fields: vec![
                                FormField::TextInput {
                                    field_id: "name".into(),
                                    label: "Name".into(),
                                    value: String::new(),
                                    validation: FormTextValidation::Any,
                                    live_warning: None,
                                },
                                FormField::CheckBox {
                                    field_id: "enabled".into(),
                                    label: "Enabled".into(),
                                    checked: false,
                                },
                            ],
                            buttons: FormButtons {
                                confirm_label: "OK".into(),
                                cancel_label: "Cancel".into(),
                                confirm_enabled: true,
                            },
                        },
                    });
                }
                self.events.push(event);
            }

            fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
                self.commands.pop_front()
            }
        }

        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(DialogHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
            target_window: window_id,
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler.clone(),
                provider,
                vec![
                    PlatformCommand::CreateButton {
                        window_id,
                        parent_control_id: None,
                        control_id: BTN_CLICK_ME,
                        text: "Open".into(),
                    },
                    PlatformCommand::ShowWindow { window_id },
                ],
            )
            .unwrap();

        let protocol_input = format!(
            "{}\n{}\n{}\n",
            serde_json::json!({
                "type": "set_dialog_responder",
                "request_id": 1,
                "script": [
                    {
                        "matcher": {
                            "kind": "message_box",
                            "window_id": window_id.raw(),
                            "title": "Notice",
                            "prompt": "Ignored",
                            "context_tag": null
                        },
                        "outcome": {
                            "kind": "message_box"
                        }
                    },
                    {
                        "matcher": {
                            "kind": "form",
                            "window_id": window_id.raw(),
                            "title": "Form",
                            "prompt": null,
                            "context_tag": "form-tag"
                        },
                        "outcome": {
                            "kind": "form",
                            "confirmed": true,
                            "field_values": [
                                {
                                    "kind": "text",
                                    "field_id": "name",
                                    "value": "Alice"
                                },
                                {
                                    "kind": "check_box",
                                    "field_id": "enabled",
                                    "checked": true
                                }
                            ]
                        }
                    }
                ]
            }),
            serde_json::json!({
                "type": "action",
                "request_id": 2,
                "action": "click",
                "window_id": window_id.raw(),
                "control_id": BTN_CLICK_ME.raw()
            }),
            serde_json::json!({
                "type": "snapshot",
                "request_id": 3
            })
        );

        let mut writer = RecordingWriter::new();
        harness
            .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
            .unwrap();

        let lines = parse_protocol_lines(&writer.into_string());
        assert_eq!(
            lines
                .iter()
                .map(|line| line["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["hello", "ok", "ok", "snapshot", "bye",]
        );
        assert_eq!(lines[0]["protocol_version"], HEADLESS_PROTOCOL_VERSION);
        assert_eq!(lines[1]["request_id"], 1);
        assert_eq!(lines[2]["request_id"], 2);
        assert_eq!(lines[3]["request_id"], 3);
        assert_eq!(
            lines[3]["model"]["dialog_requests"][0]["details"]["kind"],
            "form"
        );
        assert_eq!(
            lines[3]["model"]["dialog_requests"][0]["details"]["form"]["fields"][0]["kind"],
            "text_input"
        );
        assert_eq!(
            lines[3]["model"]["dialog_requests"][0]["details"]["form"]["buttons"]["confirm_label"],
            "OK"
        );

        let events = &handler.lock().unwrap().events;
        assert!(matches!(
            events.as_slice(),
            [
                AppEvent::ButtonClicked { control_id, .. },
                AppEvent::FormDialogCompleted {
                    confirmed: true,
                    field_values,
                    ..
                }
            ] if *control_id == BTN_CLICK_ME && field_values == &vec![
                FormFieldValue::Text {
                    field_id: "name".into(),
                    value: "Alice".into(),
                },
                FormFieldValue::CheckBox {
                    field_id: "enabled".into(),
                    checked: true,
                },
            ]
        ));
        assert!(harness.backend.dialog_responder.is_empty());
    }

    #[test]
    fn protocol_set_dialog_responder_rejects_mismatched_entry_kinds() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler,
                provider,
                vec![PlatformCommand::ShowWindow { window_id }],
            )
            .unwrap();

        let protocol_input = serde_json::json!({
            "type": "set_dialog_responder",
            "request_id": 11,
            "script": [
                {
                    "matcher": {
                        "kind": "save_file",
                        "window_id": window_id.raw(),
                        "title": "Save",
                        "prompt": null,
                        "context_tag": null
                    },
                    "outcome": {
                        "kind": "input",
                        "text": "oops"
                    }
                }
            ]
        })
        .to_string()
            + "\n";

        let mut writer = RecordingWriter::new();
        harness
            .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
            .unwrap();

        let lines = parse_protocol_lines(&writer.into_string());
        assert_eq!(lines[0]["type"], "hello");
        assert_eq!(lines[1]["type"], "error");
        assert_eq!(lines[1]["request_id"], 11);
        assert!(
            lines[1]["message"]
                .as_str()
                .unwrap()
                .contains("does not match")
        );
        assert_eq!(lines[2]["type"], "bye");
    }

    #[test]
    fn protocol_set_dialog_responder_rejects_unknown_matcher_kind() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler,
                provider,
                vec![PlatformCommand::ShowWindow { window_id }],
            )
            .unwrap();

        let protocol_input = serde_json::json!({
            "type": "set_dialog_responder",
            "request_id": 12,
            "script": [
                {
                    "matcher": {
                        "kind": "unknown_dialog",
                        "window_id": window_id.raw(),
                        "title": "Save",
                        "prompt": null,
                        "context_tag": null
                    },
                    "outcome": {
                        "kind": "save_file",
                        "path": "export.txt"
                    }
                }
            ]
        })
        .to_string()
            + "\n";

        let mut writer = RecordingWriter::new();
        harness
            .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
            .unwrap();

        let lines = parse_protocol_lines(&writer.into_string());
        assert_eq!(lines[0]["type"], "hello");
        assert_eq!(lines[1]["type"], "error");
        assert_eq!(lines[1]["request_id"], 12);
        assert!(
            lines[1]["message"]
                .as_str()
                .unwrap()
                .contains("Unknown dialog responder matcher kind")
        );
        assert_eq!(lines[2]["type"], "bye");
    }

    #[test]
    fn protocol_set_dialog_responder_reports_malformed_script_entries_with_request_id() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler,
                provider,
                vec![PlatformCommand::ShowWindow { window_id }],
            )
            .unwrap();

        let protocol_input = concat!(
            "{",
            "\"type\":\"set_dialog_responder\",",
            "\"request_id\":77,",
            "\"script\":[{",
            "\"matcher\":{",
            "\"kind\":\"form\",",
            "\"window_id\":1,",
            "\"title\":\"Form\"",
            "},",
            "\"outcome\":{",
            "\"kind\":\"form\",",
            "\"confirmed\":true,",
            "\"field_values\":[{",
            "\"field_id\":\"name\",",
            "\"value\":\"Alice\"",
            "}]", // missing `kind`
            "}",
            "}]",
            "}\n",
            "{\"type\":\"snapshot\",\"request_id\":78}\n",
        );

        let mut writer = RecordingWriter::new();
        harness
            .run_protocol(Cursor::new(protocol_input.as_bytes().to_vec()), &mut writer)
            .unwrap();

        let lines = parse_protocol_lines(&writer.into_string());
        assert_eq!(lines[0]["type"], "hello");
        assert_eq!(lines[1]["type"], "error");
        assert_eq!(lines[1]["request_id"], 77);
        assert!(lines[1]["message"].as_str().unwrap().contains("kind"));
        assert_eq!(lines[2]["type"], "snapshot");
        assert_eq!(lines[2]["request_id"], 78);
        assert_eq!(lines[3]["type"], "bye");
    }

    #[test]
    fn protocol_nonmatching_dialog_script_defaults_to_cancel() {
        struct DialogHandler {
            events: Vec<AppEvent>,
            commands: VecDeque<PlatformCommand>,
            target_window: WindowId,
        }

        impl PlatformEventHandler for DialogHandler {
            fn handle_event(&mut self, event: AppEvent) {
                if matches!(event, AppEvent::ButtonClicked { .. }) {
                    self.commands
                        .push_back(PlatformCommand::ShowSaveFileDialog {
                            window_id: self.target_window,
                            title: "Actual".into(),
                            default_filename: "export.txt".into(),
                            filter_spec: "*.txt".into(),
                            initial_dir: None,
                        });
                }
                self.events.push(event);
            }

            fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
                self.commands.pop_front()
            }
        }

        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(DialogHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
            target_window: window_id,
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler.clone(),
                provider,
                vec![
                    PlatformCommand::CreateButton {
                        window_id,
                        parent_control_id: None,
                        control_id: BTN_CLICK_ME,
                        text: "Save".into(),
                    },
                    PlatformCommand::ShowWindow { window_id },
                ],
            )
            .unwrap();

        let protocol_input = format!(
            "{}\n{}\n",
            serde_json::json!({
                "type": "set_dialog_responder",
                "request_id": 1,
                "script": [
                    {
                        "matcher": {
                            "kind": "save_file",
                            "window_id": window_id.raw(),
                            "title": "Different",
                            "prompt": null,
                            "context_tag": null
                        },
                        "outcome": {
                            "kind": "save_file",
                            "path": "scripted.txt"
                        }
                    }
                ]
            }),
            serde_json::json!({
                "type": "action",
                "request_id": 2,
                "action": "click",
                "window_id": window_id.raw(),
                "control_id": BTN_CLICK_ME.raw()
            })
        );

        let mut writer = RecordingWriter::new();
        harness
            .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
            .unwrap();

        let events = &handler.lock().unwrap().events;
        assert!(events.iter().any(|event| matches!(
            event,
            AppEvent::FileSaveDialogCompleted { result: None, .. }
        )));
        assert_eq!(harness.backend.dialog_responder.len(), 1);

        let lines = parse_protocol_lines(&writer.into_string());
        assert_eq!(
            lines
                .iter()
                .map(|line| line["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["hello", "ok", "ok", "bye"]
        );
    }

    #[test]
    fn protocol_message_box_script_entries_are_accepted_but_not_queued() {
        struct MessageBoxHandler {
            events: Vec<AppEvent>,
            commands: VecDeque<PlatformCommand>,
            target_window: WindowId,
        }

        impl PlatformEventHandler for MessageBoxHandler {
            fn handle_event(&mut self, event: AppEvent) {
                if matches!(event, AppEvent::ButtonClicked { .. }) {
                    self.commands.push_back(PlatformCommand::ShowMessageBox {
                        window_id: self.target_window,
                        title: "Notice".into(),
                        message: "Hello".into(),
                        severity: MessageSeverity::Information,
                    });
                }
                self.events.push(event);
            }

            fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
                self.commands.pop_front()
            }
        }

        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(MessageBoxHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
            target_window: window_id,
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler.clone(),
                provider,
                vec![
                    PlatformCommand::CreateButton {
                        window_id,
                        parent_control_id: None,
                        control_id: BTN_CLICK_ME,
                        text: "Notice".into(),
                    },
                    PlatformCommand::ShowWindow { window_id },
                ],
            )
            .unwrap();

        let protocol_input = format!(
            "{}\n{}\n",
            serde_json::json!({
                "type": "set_dialog_responder",
                "request_id": 1,
                "script": [
                    {
                        "matcher": {
                            "kind": "message_box",
                            "window_id": window_id.raw(),
                            "title": "Notice",
                            "prompt": "Hello",
                            "context_tag": null
                        },
                        "outcome": {
                            "kind": "message_box"
                        }
                    }
                ]
            }),
            serde_json::json!({
                "type": "action",
                "request_id": 2,
                "action": "click",
                "window_id": window_id.raw(),
                "control_id": BTN_CLICK_ME.raw()
            })
        );

        let mut writer = RecordingWriter::new();
        harness
            .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
            .unwrap();

        let events = &handler.lock().unwrap().events;
        assert!(matches!(
            events.as_slice(),
            [AppEvent::ButtonClicked { .. }]
        ));
        assert!(harness.backend.dialog_responder.is_empty());

        let lines = parse_protocol_lines(&writer.into_string());
        assert_eq!(
            lines
                .iter()
                .map(|line| line["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["hello", "ok", "ok", "bye"]
        );
    }

    #[test]
    fn define_layout_validates_rules_and_records_them() {
        let mut backend = HeadlessBackend::new("app".into());
        let window_id = backend.create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        });
        backend
            .create_control(window_id, None, ControlId::new(1), ControlKind::Panel)
            .unwrap();
        let err = backend
            .execute_platform_command(PlatformCommand::DefineLayout {
                window_id,
                rules: vec![LayoutRule {
                    control_id: ControlId::new(1),
                    parent_control_id: None,
                    dock_style: DockStyle::Top,
                    order: 0,
                    fixed_size: None,
                    margin: (1, 2, 3, 4),
                }],
            })
            .unwrap_err();
        assert!(matches!(err, PlatformError::OperationFailed(_)));

        backend
            .execute_platform_command(PlatformCommand::DefineLayout {
                window_id,
                rules: vec![LayoutRule {
                    control_id: ControlId::new(1),
                    parent_control_id: None,
                    dock_style: DockStyle::Fill,
                    order: 0,
                    fixed_size: None,
                    margin: (1, 2, 3, 4),
                }],
            })
            .unwrap();
        assert_eq!(
            backend
                .windows
                .get(&window_id.raw())
                .unwrap()
                .layout_rules
                .len(),
            1
        );
    }

    #[test]
    fn checkpoint_is_recorded_and_setup_complete_is_followed_by_event() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler.clone(),
                provider,
                vec![
                    PlatformCommand::Checkpoint {
                        label: "ready".into(),
                    },
                    PlatformCommand::SignalMainWindowUISetupComplete { window_id },
                ],
            )
            .unwrap();
        assert!(
            harness
                .backend
                .markers
                .iter()
                .any(|marker| marker == "ready")
        );
        let events = &handler.lock().unwrap().events;
        assert!(matches!(
            events.as_slice(),
            [AppEvent::MainWindowUISetupComplete { window_id: got }] if *got == window_id
        ));
    }

    #[test]
    fn snapshot_is_deterministic_and_sorted() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        harness
            .backend
            .execute_platform_command(PlatformCommand::CreateLabel {
                window_id,
                parent_control_id: None,
                control_id: ControlId::new(20),
                initial_text: "B".into(),
                class: LabelClass::Default,
            })
            .unwrap();
        harness
            .backend
            .execute_platform_command(PlatformCommand::CreateLabel {
                window_id,
                parent_control_id: None,
                control_id: ControlId::new(10),
                initial_text: "A".into(),
                class: LabelClass::Default,
            })
            .unwrap();
        let snapshot = harness.snapshot().unwrap();
        let parsed: Value = serde_json::from_str(&snapshot).unwrap();
        let controls = parsed["windows"][0]["controls"].as_array().unwrap();
        assert_eq!(controls[0]["id"], 10);
        assert_eq!(controls[1]["id"], 20);
    }

    #[test]
    fn core_commands_mutate_the_ui_model() {
        let mut backend = HeadlessBackend::new("app".into());
        let window_id = backend.create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        });

        for command in [
            PlatformCommand::SetWindowTitle {
                window_id,
                title: "Updated".into(),
            },
            PlatformCommand::ShowWindow { window_id },
            PlatformCommand::CreatePanel {
                window_id,
                parent_control_id: None,
                control_id: ControlId::new(1),
            },
            PlatformCommand::CreateButton {
                window_id,
                parent_control_id: Some(ControlId::new(1)),
                control_id: ControlId::new(2),
                text: "Click".into(),
            },
            PlatformCommand::CreateLabel {
                window_id,
                parent_control_id: Some(ControlId::new(1)),
                control_id: ControlId::new(3),
                initial_text: "Label".into(),
                class: LabelClass::StatusBar,
            },
            PlatformCommand::UpdateLabelText {
                window_id,
                control_id: ControlId::new(3),
                text: "Status".into(),
                severity: MessageSeverity::Warning,
            },
            PlatformCommand::CreateInput {
                window_id,
                parent_control_id: Some(ControlId::new(1)),
                control_id: ControlId::new(4),
                initial_text: "typed".into(),
                read_only: false,
                multiline: false,
                vertical_scroll: false,
            },
            PlatformCommand::CreateRichEdit {
                window_id,
                parent_control_id: Some(ControlId::new(1)),
                control_id: ControlId::new(5),
            },
            PlatformCommand::CreateListBox {
                window_id,
                parent_control_id: Some(ControlId::new(1)),
                control_id: ControlId::new(6),
            },
            PlatformCommand::PopulateListBox {
                window_id,
                control_id: ControlId::new(6),
                items: vec![ListBoxItemDescriptor {
                    id: ListBoxItemId::new(99),
                    badges: vec![],
                    title: "Row".into(),
                    metadata: "meta".into(),
                    enabled: false,
                }],
                badge_column_width: 42,
            },
            PlatformCommand::SetListBoxRowDensity {
                window_id,
                control_id: ControlId::new(6),
                density: ListBoxRowDensity::Compact,
            },
            PlatformCommand::SetListBoxSelection {
                window_id,
                control_id: ControlId::new(6),
                item_id: ListBoxItemId::new(99),
            },
            PlatformCommand::CreateComboBox {
                window_id,
                parent_control_id: Some(ControlId::new(1)),
                control_id: ControlId::new(7),
            },
            PlatformCommand::SetComboBoxItems {
                window_id,
                control_id: ControlId::new(7),
                items: vec!["A".into(), "B".into()],
            },
            PlatformCommand::SetComboBoxSelection {
                window_id,
                control_id: ControlId::new(7),
                selected_index: Some(1),
            },
            PlatformCommand::CreateRadioButton {
                window_id,
                parent_control_id: Some(ControlId::new(1)),
                control_id: ControlId::new(8),
                text: "Radio".into(),
                group_start: true,
            },
            PlatformCommand::SetRadioButtonChecked {
                window_id,
                control_id: ControlId::new(8),
                checked: true,
            },
            PlatformCommand::CreateCheckBox {
                window_id,
                parent_control_id: Some(ControlId::new(1)),
                control_id: ControlId::new(9),
                text: "Check".into(),
            },
            PlatformCommand::SetCheckBoxChecked {
                window_id,
                control_id: ControlId::new(9),
                checked: true,
            },
            PlatformCommand::CreateTabBar {
                window_id,
                parent_control_id: Some(ControlId::new(1)),
                control_id: ControlId::new(10),
                items: vec!["One".into(), "Two".into()],
            },
            PlatformCommand::SetTabBarItems {
                window_id,
                control_id: ControlId::new(10),
                items: vec!["Three".into(), "Four".into()],
            },
            PlatformCommand::SetTabBarSelection {
                window_id,
                control_id: ControlId::new(10),
                selected_index: 1,
            },
            PlatformCommand::CreateToggleSwitch {
                window_id,
                parent_control_id: Some(ControlId::new(1)),
                control_id: ControlId::new(11),
                label: "Toggle".into(),
                checked: false,
            },
            PlatformCommand::SetToggleSwitchState {
                window_id,
                control_id: ControlId::new(11),
                checked: true,
            },
            PlatformCommand::CreateProgressBar {
                window_id,
                parent_control_id: Some(ControlId::new(1)),
                control_id: ControlId::new(12),
            },
            PlatformCommand::SetProgressBarRange {
                window_id,
                control_id: ControlId::new(12),
                min: 10,
                max: 90,
            },
            PlatformCommand::SetProgressBarPosition {
                window_id,
                control_id: ControlId::new(12),
                position: 55,
            },
            PlatformCommand::CreateSplitter {
                window_id,
                parent_control_id: Some(ControlId::new(1)),
                control_id: ControlId::new(13),
                orientation: SplitterOrientation::Vertical,
            },
            PlatformCommand::SetControlEnabled {
                window_id,
                control_id: ControlId::new(2),
                enabled: false,
            },
            PlatformCommand::SetInputText {
                window_id,
                control_id: ControlId::new(4),
                text: "edited".into(),
            },
            PlatformCommand::SetControlText {
                window_id,
                control_id: ControlId::new(2),
                text: "Press".into(),
            },
            PlatformCommand::SetViewerContent {
                window_id,
                control_id: ControlId::new(5),
                text: "viewer".into(),
            },
            PlatformCommand::SetRichEditContent {
                window_id,
                control_id: ControlId::new(5),
                rtf_text: "{\\rtf1}".into(),
            },
            PlatformCommand::SetFocus {
                window_id,
                control_id: ControlId::new(4),
                select_all: true,
            },
            PlatformCommand::DefineLayout {
                window_id,
                rules: vec![LayoutRule {
                    control_id: ControlId::new(1),
                    parent_control_id: None,
                    dock_style: DockStyle::Fill,
                    order: 0,
                    fixed_size: None,
                    margin: (0, 0, 0, 0),
                }],
            },
        ] {
            backend.execute_platform_command(command).unwrap();
        }

        let snapshot = backend.snapshot();
        let window = &snapshot.windows[0];
        assert_eq!(window.title, "Updated");
        assert!(window.shown);
        assert_eq!(window.controls.len(), 13);
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["windows"][0]["controls"][1]["kind"], "button");
        assert_eq!(json["windows"][0]["controls"][2]["text"], "Status");
        assert_eq!(json["windows"][0]["controls"][2]["class"], "status_bar");
        assert_eq!(json["windows"][0]["controls"][2]["severity"], "warning");
        assert_eq!(json["windows"][0]["controls"][5]["density"], "compact");
        assert_eq!(
            json["windows"][0]["controls"][12]["orientation"],
            "vertical"
        );
        assert_eq!(json["windows"][0]["layout_rules"][0]["dock_style"], "fill");
        assert_eq!(json["windows"][0]["controls"][9]["selected_index"], 1);
        assert_eq!(json["windows"][0]["controls"][9]["items"][0], "Three");
        assert_eq!(json["windows"][0]["controls"][11]["position"], 55);
        assert_eq!(json["windows"][0]["controls"][5]["selected_item_id"], 99);
    }

    #[test]
    fn snapshot_uses_stable_names_for_protocol_fields() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        harness
            .backend
            .execute_platform_command(PlatformCommand::CreateLabel {
                window_id,
                parent_control_id: None,
                control_id: ControlId::new(1),
                initial_text: "Label".into(),
                class: LabelClass::StatusBar,
            })
            .unwrap();
        harness
            .backend
            .execute_platform_command(PlatformCommand::UpdateLabelText {
                window_id,
                control_id: ControlId::new(1),
                text: "Updated".into(),
                severity: MessageSeverity::Error,
            })
            .unwrap();
        harness
            .backend
            .execute_platform_command(PlatformCommand::CreateListBox {
                window_id,
                parent_control_id: None,
                control_id: ControlId::new(2),
            })
            .unwrap();
        harness
            .backend
            .execute_platform_command(PlatformCommand::SetListBoxRowDensity {
                window_id,
                control_id: ControlId::new(2),
                density: ListBoxRowDensity::Expanded,
            })
            .unwrap();
        harness
            .backend
            .execute_platform_command(PlatformCommand::DefineLayout {
                window_id,
                rules: vec![LayoutRule {
                    control_id: ControlId::new(1),
                    parent_control_id: None,
                    dock_style: DockStyle::ProportionalFill { weight: 1.0 },
                    order: 0,
                    fixed_size: None,
                    margin: (0, 0, 0, 0),
                }],
            })
            .unwrap();
        harness
            .backend
            .execute_platform_command(PlatformCommand::ShowMessageBox {
                window_id,
                title: "Notice".into(),
                message: "Hello".into(),
                severity: MessageSeverity::Information,
            })
            .unwrap();
        harness
            .backend
            .execute_platform_command(PlatformCommand::ShowFormDialog {
                window_id,
                form: FormDialogDescriptor {
                    title: "Form".into(),
                    context_tag: "ctx".into(),
                    rows: vec![FormRow::Note {
                        text: "Note".into(),
                        severity: MessageSeverity::Warning,
                    }],
                    fields: vec![FormField::TextInput {
                        field_id: "field".into(),
                        label: "Field".into(),
                        value: String::new(),
                        validation: FormTextValidation::PathSegment,
                        live_warning: None,
                    }],
                    buttons: FormButtons {
                        confirm_label: "OK".into(),
                        cancel_label: "Cancel".into(),
                        confirm_enabled: true,
                    },
                },
            })
            .unwrap();

        let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
        assert_eq!(snapshot["windows"][0]["controls"][0]["class"], "status_bar");
        assert_eq!(snapshot["windows"][0]["controls"][0]["severity"], "error");
        assert_eq!(snapshot["windows"][0]["controls"][1]["density"], "expanded");
        assert_eq!(
            snapshot["windows"][0]["layout_rules"][0]["dock_style"],
            "proportional_fill"
        );
        assert_eq!(snapshot["dialog_requests"][0]["kind"], "message_box");
        assert_eq!(
            snapshot["dialog_requests"][0]["details"]["severity"],
            "information"
        );
        assert_eq!(
            snapshot["dialog_requests"][1]["details"]["form"]["rows"][0]["severity"],
            "warning"
        );
        assert_eq!(
            snapshot["dialog_requests"][1]["details"]["form"]["fields"][0]["validation"],
            "path_segment"
        );
    }

    #[test]
    fn treeview_commands_update_state_and_snapshot_order() {
        let mut backend = HeadlessBackend::new("app".into());
        let window_id = backend.create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        });
        backend
            .execute_platform_command(PlatformCommand::CreateTreeView {
                window_id,
                parent_control_id: None,
                control_id: ControlId::new(1),
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::PopulateTreeView {
                window_id,
                control_id: ControlId::new(1),
                items: vec![TreeItemDescriptor {
                    id: TreeItemId::new(10),
                    text: "Parent".into(),
                    is_folder: true,
                    state: CheckState::Unchecked,
                    style_override: Some(StyleId::TreeItemDisabled),
                    children: vec![
                        TreeItemDescriptor {
                            id: TreeItemId::new(11),
                            text: "Child B".into(),
                            is_folder: false,
                            state: CheckState::Checked,
                            style_override: None,
                            children: vec![],
                        },
                        TreeItemDescriptor {
                            id: TreeItemId::new(12),
                            text: "Child A".into(),
                            is_folder: false,
                            state: CheckState::Unchecked,
                            style_override: Some(StyleId::DefaultText),
                            children: vec![],
                        },
                    ],
                }],
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::UpdateTreeItemText {
                window_id,
                control_id: ControlId::new(1),
                item_id: TreeItemId::new(12),
                text: "Child A+".into(),
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::UpdateTreeItemVisualState {
                window_id,
                control_id: ControlId::new(1),
                item_id: TreeItemId::new(11),
                new_state: CheckState::Hidden,
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::SetTreeViewSelection {
                window_id,
                control_id: ControlId::new(1),
                item_id: TreeItemId::new(12),
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::ExpandAllTreeItems {
                window_id,
                control_id: ControlId::new(1),
            })
            .unwrap();

        let snapshot =
            serde_json::from_str::<Value>(&serde_json::to_string(&backend.snapshot()).unwrap())
                .unwrap();
        let tree = &snapshot["windows"][0]["controls"][0];
        assert_eq!(tree["kind"], "tree_view");
        assert_eq!(tree["selected_item_id"], 12);
        assert_eq!(tree["items"][0]["style_override"], "TreeItemDisabled");
        assert_eq!(tree["items"][0]["children"][0]["text"], "Child B");
        assert_eq!(tree["items"][0]["children"][1]["text"], "Child A+");
        assert_eq!(tree["items"][0]["children"][0]["expanded"], true);
        assert_eq!(tree["items"][0]["children"][1]["expanded"], true);
        assert_eq!(tree["items"][0]["children"][0]["state"], "Hidden");
        assert!(matches!(
            backend.follow_up_events.pop_front(),
            Some(AppEvent::TreeViewItemSelectionChanged {
                window_id: got_window_id,
                item_id: got_item_id,
            }) if got_window_id == window_id && got_item_id == TreeItemId::new(12)
        ));
    }

    #[test]
    fn hidden_tree_toggle_is_silent() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler.clone(),
                provider,
                vec![
                    PlatformCommand::CreateTreeView {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(1),
                    },
                    PlatformCommand::PopulateTreeView {
                        window_id,
                        control_id: ControlId::new(1),
                        items: vec![TreeItemDescriptor {
                            id: TreeItemId::new(7),
                            text: "Row".into(),
                            is_folder: false,
                            state: CheckState::Hidden,
                            style_override: None,
                            children: vec![],
                        }],
                    },
                    PlatformCommand::ShowWindow { window_id },
                ],
            )
            .unwrap();

        harness
            .toggle_tree_item(window_id, ControlId::new(1), TreeItemId::new(7))
            .unwrap();
        assert!(handler.lock().unwrap().events.is_empty());
        let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
        assert_eq!(
            snapshot["windows"][0]["controls"][0]["items"][0]["state"],
            "Hidden"
        );
    }

    #[test]
    fn chart_menu_style_and_scroll_commands_update_snapshot() {
        let mut backend = HeadlessBackend::new("app".into());
        let window_id = backend.create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        });
        backend
            .execute_platform_command(PlatformCommand::CreateButton {
                window_id,
                parent_control_id: None,
                control_id: ControlId::new(1),
                text: "Button".into(),
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::CreateChart {
                window_id,
                parent_control_id: None,
                control_id: ControlId::new(2),
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::CreateTabBar {
                window_id,
                parent_control_id: None,
                control_id: ControlId::new(3),
                items: vec!["One".into(), "Two".into()],
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::CreateToggleSwitch {
                window_id,
                parent_control_id: None,
                control_id: ControlId::new(4),
                label: "Toggle".into(),
                checked: false,
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::SetChartData {
                window_id,
                control_id: ControlId::new(2),
                data: ChartDataPacket {
                    lines: vec![ChartLineData {
                        label: "Alpha".into(),
                        weekly_counts: vec![1, 2, 3],
                        color: 0x00FF00,
                        end_label: Some("A".into()),
                        emphasis: ChartLineEmphasis::Secondary,
                    }],
                    week_labels: vec!["W1".into(), "W2".into(), "W3".into()],
                    is_loading: false,
                    show_x_axis_labels: true,
                    show_y_axis_labels: true,
                    show_end_labels: true,
                },
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::CreateMainMenu {
                window_id,
                menu_items: vec![MenuItemConfig {
                    action: None,
                    text: "&File".into(),
                    children: vec![MenuItemConfig {
                        action: Some(MenuActionId::new(42)),
                        text: "Exit".into(),
                        children: vec![],
                    }],
                }],
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::DefineStyle {
                style_id: StyleId::PrimaryButton,
                style: ControlStyle::default(),
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::ApplyStyleToControl {
                window_id,
                control_id: ControlId::new(1),
                style_id: StyleId::PrimaryButton,
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::SetTabBarStyle {
                window_id,
                control_id: ControlId::new(3),
                background_color: Color::default(),
                text_color: Color::default(),
                accent_color: Color::default(),
                font: None,
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::SetToggleSwitchStyle {
                window_id,
                control_id: ControlId::new(4),
                background: Color::default(),
                pill_off: Color::default(),
                pill_on: Color::default(),
                knob: Color::default(),
                text: Color::default(),
            })
            .unwrap();
        backend
            .execute_platform_command(PlatformCommand::SetScrollPosition {
                window_id,
                control_id: ControlId::new(1),
                vertical_pos: 25,
                horizontal_pos: 75,
            })
            .unwrap();

        let snapshot =
            serde_json::from_str::<Value>(&serde_json::to_string(&backend.snapshot()).unwrap())
                .unwrap();
        let window = &snapshot["windows"][0];
        assert_eq!(window["menu"][0]["children"][0]["action"], 42);
        let chart = &window["controls"][1];
        assert_eq!(chart["kind"], "chart");
        assert_eq!(chart["data"]["lines"][0]["emphasis"], "Secondary");
        assert!(chart["data"]["lines"][0].get("color").is_none());
        assert_eq!(window["controls"][0]["style_id"], "PrimaryButton");
        assert_eq!(window["controls"][0]["scroll_vertical"], 25);
        assert_eq!(window["controls"][0]["scroll_horizontal"], 75);
    }

    #[test]
    fn scroll_action_emits_event_and_rejects_list_box() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler.clone(),
                provider,
                vec![
                    PlatformCommand::CreateInput {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(1),
                        initial_text: String::new(),
                        read_only: false,
                        multiline: false,
                        vertical_scroll: true,
                    },
                    PlatformCommand::CreateListBox {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(2),
                    },
                    PlatformCommand::ShowWindow { window_id },
                ],
            )
            .unwrap();

        harness
            .scroll(window_id, ControlId::new(1), 33, 44)
            .unwrap();
        let events = &handler.lock().unwrap().events;
        assert!(matches!(
            events.last(),
            Some(AppEvent::ControlScrolled {
                vertical_pos: 33,
                horizontal_pos: 44,
                ..
            })
        ));
        let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
        assert_eq!(snapshot["windows"][0]["controls"][0]["scroll_vertical"], 33);
        assert_eq!(
            snapshot["windows"][0]["controls"][0]["scroll_horizontal"],
            44
        );
        assert!(harness.scroll(window_id, ControlId::new(2), 1, 1).is_err());
    }

    #[test]
    fn radio_buttons_are_scoped_by_group_start() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler,
                provider,
                vec![
                    PlatformCommand::CreatePanel {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(1),
                    },
                    PlatformCommand::CreateRadioButton {
                        window_id,
                        parent_control_id: Some(ControlId::new(1)),
                        control_id: ControlId::new(10),
                        text: "A".into(),
                        group_start: true,
                    },
                    PlatformCommand::CreateRadioButton {
                        window_id,
                        parent_control_id: Some(ControlId::new(1)),
                        control_id: ControlId::new(11),
                        text: "B".into(),
                        group_start: false,
                    },
                    PlatformCommand::CreateRadioButton {
                        window_id,
                        parent_control_id: Some(ControlId::new(1)),
                        control_id: ControlId::new(12),
                        text: "C".into(),
                        group_start: true,
                    },
                    PlatformCommand::CreateRadioButton {
                        window_id,
                        parent_control_id: Some(ControlId::new(1)),
                        control_id: ControlId::new(13),
                        text: "D".into(),
                        group_start: false,
                    },
                    PlatformCommand::ShowWindow { window_id },
                ],
            )
            .unwrap();

        harness.select_radio(window_id, ControlId::new(11)).unwrap();
        harness.select_radio(window_id, ControlId::new(13)).unwrap();
        let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
        assert_eq!(snapshot["windows"][0]["controls"][1]["checked"], false);
        assert_eq!(snapshot["windows"][0]["controls"][2]["checked"], true);
        assert_eq!(snapshot["windows"][0]["controls"][3]["checked"], false);
        assert_eq!(snapshot["windows"][0]["controls"][4]["checked"], true);
    }

    #[test]
    fn semantic_actions_emit_events_and_mutate_state() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler.clone(),
                provider,
                vec![
                    PlatformCommand::CreateButton {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(1),
                        text: "Click".into(),
                    },
                    PlatformCommand::CreateInput {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(2),
                        initial_text: String::new(),
                        read_only: false,
                        multiline: false,
                        vertical_scroll: false,
                    },
                    PlatformCommand::CreateListBox {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(3),
                    },
                    PlatformCommand::PopulateListBox {
                        window_id,
                        control_id: ControlId::new(3),
                        items: vec![ListBoxItemDescriptor {
                            id: ListBoxItemId::new(7),
                            badges: vec![],
                            title: "Row".into(),
                            metadata: String::new(),
                            enabled: false,
                        }],
                        badge_column_width: 0,
                    },
                    PlatformCommand::CreateComboBox {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(4),
                    },
                    PlatformCommand::SetComboBoxItems {
                        window_id,
                        control_id: ControlId::new(4),
                        items: vec!["A".into(), "B".into()],
                    },
                    PlatformCommand::CreateTabBar {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(5),
                        items: vec!["One".into(), "Two".into()],
                    },
                    PlatformCommand::CreateCheckBox {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(6),
                        text: "Check".into(),
                    },
                    PlatformCommand::CreateRadioButton {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(7),
                        text: "Radio".into(),
                        group_start: true,
                    },
                    PlatformCommand::CreateToggleSwitch {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(8),
                        label: "Toggle".into(),
                        checked: false,
                    },
                    PlatformCommand::ShowWindow { window_id },
                ],
            )
            .unwrap();

        harness
            .set_text(window_id, ControlId::new(2), "typed")
            .unwrap();
        harness
            .select_row(window_id, ControlId::new(3), ListBoxItemId::new(7))
            .unwrap();
        harness
            .select_combo(window_id, ControlId::new(4), 1)
            .unwrap();
        harness.select_tab(window_id, ControlId::new(5), 1).unwrap();
        harness.toggle(window_id, ControlId::new(6)).unwrap();
        harness.select_radio(window_id, ControlId::new(7)).unwrap();
        harness.toggle(window_id, ControlId::new(8)).unwrap();
        harness.click(window_id, ControlId::new(1)).unwrap();

        let events = &handler.lock().unwrap().events;
        assert!(matches!(events[0], AppEvent::InputTextChanged { .. }));
        assert!(matches!(
            events[1],
            AppEvent::ListBoxItemSelectionChanged { .. }
        ));
        assert!(matches!(
            events[2],
            AppEvent::ComboBoxSelectionChanged { .. }
        ));
        assert!(matches!(events[3], AppEvent::TabBarSelectionChanged { .. }));
        assert!(matches!(
            events[4],
            AppEvent::CheckBoxToggled { checked: true, .. }
        ));
        assert!(matches!(events[5], AppEvent::RadioButtonSelected { .. }));
        assert!(matches!(
            events[6],
            AppEvent::ToggleSwitchToggled { checked: true, .. }
        ));
        assert!(matches!(events[7], AppEvent::ButtonClicked { .. }));
        let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
        assert_eq!(snapshot["windows"][0]["controls"][1]["text"], "typed");
        assert_eq!(snapshot["windows"][0]["controls"][2]["selected_item_id"], 7);
        assert_eq!(snapshot["windows"][0]["controls"][3]["selected_index"], 1);
        assert_eq!(snapshot["windows"][0]["controls"][4]["selected_index"], 1);
        assert_eq!(snapshot["windows"][0]["controls"][5]["checked"], true);
        assert_eq!(snapshot["windows"][0]["controls"][6]["checked"], true);
        assert_eq!(snapshot["windows"][0]["controls"][7]["checked"], true);
    }

    #[test]
    fn semantic_action_reaction_commands_apply_within_same_call() {
        struct ReactionHandler {
            events: Vec<AppEvent>,
            command_queue: VecDeque<PlatformCommand>,
            target_window: WindowId,
        }

        impl PlatformEventHandler for ReactionHandler {
            fn handle_event(&mut self, event: AppEvent) {
                self.events.push(event);
                if matches!(self.events.last(), Some(AppEvent::ButtonClicked { .. })) {
                    self.command_queue
                        .push_back(PlatformCommand::SetWindowTitle {
                            window_id: self.target_window,
                            title: "Updated".into(),
                        });
                }
            }

            fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
                self.command_queue.pop_front()
            }
        }

        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(ReactionHandler {
            events: Vec::new(),
            command_queue: VecDeque::new(),
            target_window: window_id,
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler,
                provider,
                vec![
                    PlatformCommand::CreateButton {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(1),
                        text: "Click".into(),
                    },
                    PlatformCommand::ShowWindow { window_id },
                ],
            )
            .unwrap();

        harness.click(window_id, ControlId::new(1)).unwrap();
        let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
        assert_eq!(snapshot["windows"][0]["title"], "Updated");
    }

    #[test]
    fn inject_raw_can_drive_user_requested_close_flow() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::from([PlatformCommand::CloseWindow { window_id }]),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler,
                provider,
                vec![PlatformCommand::ShowWindow { window_id }],
            )
            .unwrap();
        harness
            .inject_raw(AppEvent::WindowCloseRequestedByUser { window_id })
            .unwrap();

        let snapshot = harness.snapshot().unwrap();
        let parsed: Value = serde_json::from_str(&snapshot).unwrap();
        assert!(parsed["windows"][0]["closed"].as_bool().unwrap());
    }

    #[test]
    fn semantic_actions_reject_hidden_and_read_only_inputs() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness
            .start(
                handler,
                provider,
                vec![
                    PlatformCommand::CreateInput {
                        window_id,
                        parent_control_id: None,
                        control_id: ControlId::new(1),
                        initial_text: String::new(),
                        read_only: true,
                        multiline: false,
                        vertical_scroll: false,
                    },
                    PlatformCommand::ShowWindow { window_id },
                ],
            )
            .unwrap();

        assert!(
            harness
                .set_text(window_id, ControlId::new(1), "nope")
                .is_err()
        );
        assert!(
            harness
                .select_combo(window_id, ControlId::new(1), 0)
                .is_err()
        );
        assert!(harness.select_tab(window_id, ControlId::new(1), 0).is_err());
    }

    #[test]
    fn wait_for_times_out_when_marker_missing() {
        let mut harness = HeadlessHarness::new("app");
        let err = harness
            .wait_for("missing", Duration::from_millis(1))
            .unwrap_err();
        assert!(matches!(err, PlatformError::OperationFailed(_)));
    }

    #[test]
    fn wait_until_returns_immediately_when_condition_is_already_true() {
        let (mut harness, window_id, _handler, _provider) =
            started_harness(vec![PlatformCommand::ShowWindow {
                window_id: WindowId::new(1),
            }]);

        harness
            .wait_until(
                |snapshot| {
                    snapshot["windows"][0]["shown"].as_bool() == Some(true)
                        && snapshot["windows"][0]["title"] == "Window"
                },
                Duration::from_millis(5),
            )
            .unwrap();

        assert_eq!(window_id.raw(), 1);
    }

    #[test]
    fn wait_until_pumps_until_condition_becomes_true() {
        let (mut harness, window_id, _handler, _provider) =
            started_harness(vec![PlatformCommand::ShowWindow {
                window_id: WindowId::new(1),
            }]);
        harness
            .backend
            .command_queue
            .push_back(PlatformCommand::SetWindowTitle {
                window_id,
                title: "Updated".into(),
            });

        harness
            .wait_until(
                |snapshot| snapshot["windows"][0]["title"] == "Updated",
                Duration::from_millis(5),
            )
            .unwrap();
    }

    #[test]
    fn wait_until_times_out_when_condition_is_never_true() {
        let mut harness = HeadlessHarness::new("app");
        let err = harness
            .wait_until(
                |snapshot| {
                    snapshot["windows"]
                        .as_array()
                        .is_some_and(|windows| !windows.is_empty())
                },
                Duration::from_millis(1),
            )
            .unwrap_err();
        assert!(matches!(err, PlatformError::OperationFailed(_)));
    }

    #[test]
    fn inject_raw_requires_an_event_handler() {
        let mut harness = HeadlessHarness::new("app");
        let err = harness
            .inject_raw(AppEvent::WindowCloseRequestedByUser {
                window_id: WindowId::new(1),
            })
            .unwrap_err();
        assert!(matches!(err, PlatformError::OperationFailed(_)));
    }

    #[test]
    fn dialog_commands_emit_completions_and_record_requests() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness.set_dialog_responder(vec![
            DialogScriptEntry {
                matcher: DialogMatcher {
                    kind: DialogKind::SaveFile,
                    window_id: Some(window_id),
                    title: Some("Save".into()),
                    prompt: None,
                    context_tag: None,
                },
                outcome: DialogOutcome::SaveFile {
                    result: Some(PathBuf::from("export.txt")),
                },
            },
            DialogScriptEntry {
                matcher: DialogMatcher {
                    kind: DialogKind::OpenFile,
                    window_id: Some(window_id),
                    title: Some("Open".into()),
                    prompt: None,
                    context_tag: None,
                },
                outcome: DialogOutcome::OpenFile {
                    result: Some(PathBuf::from("profile.json")),
                },
            },
            DialogScriptEntry {
                matcher: DialogMatcher {
                    kind: DialogKind::ProfileSelection,
                    window_id: Some(window_id),
                    title: Some("Profiles".into()),
                    prompt: Some("Choose".into()),
                    context_tag: None,
                },
                outcome: DialogOutcome::ProfileSelection {
                    chosen_profile_name: Some("Default".into()),
                    create_new_requested: false,
                    user_cancelled: false,
                },
            },
            DialogScriptEntry {
                matcher: DialogMatcher {
                    kind: DialogKind::Input,
                    window_id: Some(window_id),
                    title: Some("Prompt".into()),
                    prompt: Some("Enter value".into()),
                    context_tag: Some("ctx".into()),
                },
                outcome: DialogOutcome::Input {
                    text: Some("typed".into()),
                },
            },
            DialogScriptEntry {
                matcher: DialogMatcher {
                    kind: DialogKind::ExcludePatterns,
                    window_id: Some(window_id),
                    title: Some("Patterns".into()),
                    prompt: None,
                    context_tag: None,
                },
                outcome: DialogOutcome::ExcludePatterns {
                    saved: true,
                    patterns: "target/\n*.tmp".into(),
                },
            },
            DialogScriptEntry {
                matcher: DialogMatcher {
                    kind: DialogKind::Form,
                    window_id: Some(window_id),
                    title: Some("Form".into()),
                    prompt: None,
                    context_tag: Some("form-tag".into()),
                },
                outcome: DialogOutcome::Form {
                    confirmed: true,
                    field_values: vec![
                        FormFieldValue::Text {
                            field_id: "name".into(),
                            value: "Alice".into(),
                        },
                        FormFieldValue::CheckBox {
                            field_id: "enabled".into(),
                            checked: true,
                        },
                    ],
                },
            },
            DialogScriptEntry {
                matcher: DialogMatcher {
                    kind: DialogKind::FolderPicker,
                    window_id: Some(window_id),
                    title: Some("Folder".into()),
                    prompt: None,
                    context_tag: None,
                },
                outcome: DialogOutcome::FolderPicker {
                    path: Some(PathBuf::from("C:/tmp")),
                },
            },
        ]);
        harness
            .start(
                handler.clone(),
                provider,
                vec![
                    PlatformCommand::ShowSaveFileDialog {
                        window_id,
                        title: "Save".into(),
                        default_filename: "export.txt".into(),
                        filter_spec: "*.txt".into(),
                        initial_dir: Some(PathBuf::from("C:/temp")),
                    },
                    PlatformCommand::ShowOpenFileDialog {
                        window_id,
                        title: "Open".into(),
                        filter_spec: "*.json".into(),
                        initial_dir: None,
                    },
                    PlatformCommand::ShowProfileSelectionDialog {
                        window_id,
                        available_profiles: vec!["Default".into()],
                        title: "Profiles".into(),
                        prompt: "Choose".into(),
                    },
                    PlatformCommand::ShowInputDialog {
                        window_id,
                        title: "Prompt".into(),
                        prompt: "Enter value".into(),
                        default_text: Some("seed".into()),
                        context_tag: Some("ctx".into()),
                    },
                    PlatformCommand::ShowExcludePatternsDialog {
                        window_id,
                        title: "Patterns".into(),
                        patterns: "target/\n*.tmp".into(),
                    },
                    PlatformCommand::ShowFormDialog {
                        window_id,
                        form: FormDialogDescriptor {
                            title: "Form".into(),
                            context_tag: "form-tag".into(),
                            rows: vec![
                                FormRow::ReadOnlyText {
                                    label: "Info".into(),
                                    value: "Value".into(),
                                },
                                FormRow::Note {
                                    text: "Note".into(),
                                    severity: MessageSeverity::Information,
                                },
                            ],
                            fields: vec![
                                FormField::TextInput {
                                    field_id: "name".into(),
                                    label: "Name".into(),
                                    value: String::new(),
                                    validation: FormTextValidation::Any,
                                    live_warning: None,
                                },
                                FormField::CheckBox {
                                    field_id: "enabled".into(),
                                    label: "Enabled".into(),
                                    checked: false,
                                },
                            ],
                            buttons: FormButtons {
                                confirm_label: "OK".into(),
                                cancel_label: "Cancel".into(),
                                confirm_enabled: true,
                            },
                        },
                    },
                    PlatformCommand::ShowFolderPickerDialog {
                        window_id,
                        title: "Folder".into(),
                        initial_dir: Some(PathBuf::from("C:/tmp")),
                    },
                    PlatformCommand::ShowMessageBox {
                        window_id,
                        title: "Notice".into(),
                        message: "Hello".into(),
                        severity: MessageSeverity::Information,
                    },
                ],
            )
            .unwrap();

        let events = &handler.lock().unwrap().events;
        assert_eq!(events.len(), 7);
        assert!(matches!(
            events[0],
            AppEvent::FileSaveDialogCompleted {
                window_id: got,
                result: Some(_),
            } if got == window_id
        ));
        assert!(matches!(
            events[1],
            AppEvent::FileOpenProfileDialogCompleted {
                window_id: got,
                result: Some(_),
            } if got == window_id
        ));
        assert!(matches!(
            events[2],
            AppEvent::ProfileSelectionDialogCompleted {
                chosen_profile_name: Some(_),
                create_new_requested: false,
                user_cancelled: false,
                ..
            }
        ));
        assert!(matches!(
            events[3],
            AppEvent::GenericInputDialogCompleted {
                text: Some(_),
                context_tag: Some(_),
                ..
            }
        ));
        assert!(matches!(
            &events[4],
            AppEvent::ExcludePatternsDialogCompleted {
                saved: true,
                patterns,
                ..
            } if patterns == "target/\n*.tmp"
        ));
        assert!(matches!(
            &events[5],
            AppEvent::FormDialogCompleted {
                context_tag,
                confirmed: true,
                field_values,
                ..
            } if context_tag == "form-tag" && field_values.len() == 2
        ));
        assert!(matches!(
            events[6],
            AppEvent::FolderPickerDialogCompleted { path: Some(_), .. }
        ));

        let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
        assert_eq!(snapshot["dialog_requests"].as_array().unwrap().len(), 8);
        assert_eq!(
            snapshot["dialog_requests"][0]["details"]["kind"],
            "save_file"
        );
        assert_eq!(
            snapshot["dialog_requests"][0]["details"]["default_filename"],
            "export.txt"
        );
        assert_eq!(
            snapshot["dialog_requests"][2]["details"]["available_profiles"][0],
            "Default"
        );
        assert_eq!(
            snapshot["dialog_requests"][5]["details"]["form"]["fields"][0]["kind"],
            "text_input"
        );
        assert_eq!(
            snapshot["dialog_requests"][5]["details"]["form"]["buttons"]["confirm_label"],
            "OK"
        );
        assert_eq!(snapshot["dialog_requests"][7]["kind"], "message_box");
        assert_eq!(
            snapshot["dialog_requests"][7]["details"]["kind"],
            "message_box"
        );
        assert_eq!(
            snapshot["dialog_requests"][7]["details"]["severity"],
            "information"
        );
    }

    #[test]
    fn dialog_responder_uses_ordered_matches_and_field_constraints() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness.set_dialog_responder(vec![
            DialogScriptEntry {
                matcher: DialogMatcher {
                    kind: DialogKind::Input,
                    window_id: Some(window_id),
                    title: Some("First".into()),
                    prompt: Some("Prompt 1".into()),
                    context_tag: Some("tag".into()),
                },
                outcome: DialogOutcome::Input {
                    text: Some("one".into()),
                },
            },
            DialogScriptEntry {
                matcher: DialogMatcher {
                    kind: DialogKind::Input,
                    window_id: Some(window_id),
                    title: Some("Second".into()),
                    prompt: Some("Prompt 2".into()),
                    context_tag: Some("tag".into()),
                },
                outcome: DialogOutcome::Input {
                    text: Some("two".into()),
                },
            },
        ]);
        harness
            .start(
                handler.clone(),
                provider,
                vec![
                    PlatformCommand::ShowInputDialog {
                        window_id,
                        title: "First".into(),
                        prompt: "Prompt 1".into(),
                        default_text: None,
                        context_tag: Some("tag".into()),
                    },
                    PlatformCommand::ShowInputDialog {
                        window_id,
                        title: "Second".into(),
                        prompt: "Prompt 2".into(),
                        default_text: None,
                        context_tag: Some("tag".into()),
                    },
                ],
            )
            .unwrap();

        let events = &handler.lock().unwrap().events;
        assert!(matches!(
            events.as_slice(),
            [
                AppEvent::GenericInputDialogCompleted { text: Some(first), .. },
                AppEvent::GenericInputDialogCompleted { text: Some(second), .. }
            ] if first == "one" && second == "two"
        ));
    }

    #[test]
    fn unmatched_dialog_defaults_to_cancel_without_consuming_responder() {
        let mut harness = HeadlessHarness::new("app");
        let window_id = harness
            .create_window(WindowConfig {
                title: "Window",
                width: 320,
                height: 240,
            })
            .unwrap();
        let handler = Arc::new(Mutex::new(TestHandler {
            events: Vec::new(),
            commands: VecDeque::new(),
        }));
        let provider = Arc::new(Mutex::new(SilentProvider));
        harness.set_dialog_responder(vec![DialogScriptEntry {
            matcher: DialogMatcher {
                kind: DialogKind::SaveFile,
                window_id: Some(window_id),
                title: Some("Different".into()),
                prompt: None,
                context_tag: None,
            },
            outcome: DialogOutcome::SaveFile {
                result: Some(PathBuf::from("scripted.txt")),
            },
        }]);
        harness
            .start(
                handler.clone(),
                provider,
                vec![PlatformCommand::ShowSaveFileDialog {
                    window_id,
                    title: "Actual".into(),
                    default_filename: "export.txt".into(),
                    filter_spec: "*.txt".into(),
                    initial_dir: None,
                }],
            )
            .unwrap();

        let events = &handler.lock().unwrap().events;
        assert!(matches!(
            events.as_slice(),
            [AppEvent::FileSaveDialogCompleted { result: None, .. }]
        ));
        assert_eq!(harness.backend.dialog_responder.len(), 1);
    }
}
