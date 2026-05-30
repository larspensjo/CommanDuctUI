use crate::{
    AppEvent, BadgeDescriptor, ControlId, DockStyle, LabelClass, LayoutRule, ListBoxItemDescriptor,
    ListBoxItemId, ListBoxRowDensity, MessageSeverity, PlatformCommand, PlatformError,
    PlatformEventHandler, PlatformResult, SplitterOrientation, UiStateProvider, WindowConfig,
    WindowId,
};
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type SharedHandler = Arc<Mutex<dyn PlatformEventHandler>>;
type SharedProvider = Arc<Mutex<dyn UiStateProvider>>;

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

    pub fn snapshot(&self) -> PlatformResult<String> {
        serde_json::to_string_pretty(&self.backend.snapshot()).map_err(|err| {
            PlatformError::OperationFailed(format!("Failed to serialize headless snapshot: {err}"))
        })
    }

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
}

#[derive(Debug)]
struct HeadlessBackend {
    app_name: String,
    next_window_id: usize,
    windows: BTreeMap<usize, WindowState>,
    markers: Vec<String>,
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
            markers: Vec::new(),
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
            PlatformCommand::CreateTreeView { .. }
            | PlatformCommand::PopulateTreeView { .. }
            | PlatformCommand::UpdateTreeItemVisualState { .. }
            | PlatformCommand::UpdateTreeItemText { .. }
            | PlatformCommand::CreateMainMenu { .. }
            | PlatformCommand::CreateChart { .. }
            | PlatformCommand::ShowSaveFileDialog { .. }
            | PlatformCommand::ShowOpenFileDialog { .. }
            | PlatformCommand::ShowProfileSelectionDialog { .. }
            | PlatformCommand::ShowInputDialog { .. }
            | PlatformCommand::ShowExcludePatternsDialog { .. }
            | PlatformCommand::ShowFormDialog { .. }
            | PlatformCommand::ShowMessageBox { .. }
            | PlatformCommand::ShowFolderPickerDialog { .. }
            | PlatformCommand::ExpandVisibleTreeItems { .. }
            | PlatformCommand::ExpandAllTreeItems { .. }
            | PlatformCommand::RedrawTreeItem { .. }
            | PlatformCommand::SetChartData { .. }
            | PlatformCommand::SetTabBarStyle { .. }
            | PlatformCommand::DefineStyle { .. }
            | PlatformCommand::ApplyStyleToControl { .. }
            | PlatformCommand::SetToggleSwitchStyle { .. }
            | PlatformCommand::SetScrollPosition { .. }
            | PlatformCommand::SetTreeViewSelection { .. } => {
                Err(Self::unsupported_command(command))
            }
        }
    }

    fn unsupported_command(command: PlatformCommand) -> PlatformError {
        PlatformError::OperationFailed(format!("Headless backend does not support {command:?}"))
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

#[derive(Debug, Serialize)]
struct HeadlessSnapshot {
    app_name: String,
    quitting: bool,
    markers: Vec<String>,
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
            dock_style: format!("{:?}", rule.dock_style),
            order: rule.order,
            fixed_size: rule.fixed_size,
            margin: [rule.margin.0, rule.margin.1, rule.margin.2, rule.margin.3],
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
                class: format!("{class:?}"),
                severity: format!("{severity:?}"),
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
                density: format!("{density:?}"),
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
                orientation: format!("{orientation:?}"),
            },
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
            style: format!("{:?}", badge.style),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TreeItemId;
    use serde_json::Value;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

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
        assert_eq!(json["windows"][0]["controls"][2]["severity"], "Warning");
        assert_eq!(json["windows"][0]["controls"][9]["selected_index"], 1);
        assert_eq!(json["windows"][0]["controls"][9]["items"][0], "Three");
        assert_eq!(json["windows"][0]["controls"][11]["position"], 55);
        assert_eq!(json["windows"][0]["controls"][5]["selected_item_id"], 99);
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
}
