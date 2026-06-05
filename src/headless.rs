use crate::{
    AppEvent, ControlId, KeyModifiers, ListBoxItemId, MenuActionId, MessageSeverity,
    PlatformCommand, PlatformError, PlatformEventHandler, PlatformResult, TreeItemId,
    UiStateProvider, WindowConfig, WindowId,
};
use serde::Serialize;
use serde_json::Value;
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

mod protocol;

use protocol::{
    HEADLESS_PROTOCOL_VERSION, ProtocolActionRequest, ProtocolDialogScriptEntry, ProtocolRequest,
    ProtocolRequestEnvelope, ProtocolResponse,
};

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
            ProtocolActionRequest::ScrollListBox {
                window_id,
                control_id,
                position,
            } => self.scroll_listbox(
                WindowId::new(window_id),
                ControlId::new(control_id),
                position,
            ),
            ProtocolActionRequest::KeyListBox {
                window_id,
                control_id,
                key_code,
            } => self.key_listbox(
                WindowId::new(window_id),
                ControlId::new(control_id),
                key_code,
            ),
            ProtocolActionRequest::KeyInput {
                window_id,
                control_id,
                key_code,
                ctrl,
                shift,
                alt,
            } => self.key_input(
                WindowId::new(window_id),
                ControlId::new(control_id),
                key_code,
                KeyModifiers { ctrl, shift, alt },
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

    pub fn scroll_listbox(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        position: u32,
    ) -> PlatformResult<()> {
        self.backend
            .set_list_box_scroll_position(window_id, control_id, position)?;
        self.enqueue_follow_up_event(AppEvent::ListBoxScrolled {
            window_id,
            control_id,
            position,
        });
        self.pump()
    }

    pub fn key_listbox(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        key_code: u16,
    ) -> PlatformResult<()> {
        self.backend
            .validate_visible_enabled_list_box(window_id, control_id)?;
        self.enqueue_follow_up_event(AppEvent::ListBoxItemKeyDown {
            window_id,
            control_id,
            key_code,
        });
        self.pump()
    }

    pub fn key_input(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        key_code: u16,
        modifiers: KeyModifiers,
    ) -> PlatformResult<()> {
        self.backend
            .validate_visible_enabled_input_keydown(window_id, control_id)?;
        self.enqueue_follow_up_event(AppEvent::InputKeyDown {
            window_id,
            control_id,
            key_code,
            modifiers,
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

mod backend;

use backend::HeadlessBackend;

mod state;

mod snapshot;

#[cfg(test)]
mod flight_recorder;
#[cfg(test)]
mod tests;
