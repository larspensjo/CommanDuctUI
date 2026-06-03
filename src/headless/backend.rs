use super::snapshot::{DialogRequestSnapshot, HeadlessSnapshot, ProtocolSnapshot};
use super::state::{
    ChartDataState, ControlKind, ControlState, MenuNode, TreeItemNode, WindowState,
    find_menu_action, tree_items_contain_id, tree_items_find, tree_items_find_mut,
};
use super::{DialogKind, DialogOutcome, DialogRequest, DialogRequestDetails, DialogScriptEntry};
use crate::{
    AppEvent, ChartDataPacket, CheckState, ControlId, ControlStyle, LayoutRule,
    ListBoxItemDescriptor, ListBoxItemId, ListBoxRowDensity, MenuActionId, MessageSeverity,
    PlatformCommand, PlatformError, PlatformResult, StyleId, TreeItemDescriptor, TreeItemId,
    WindowConfig, WindowId,
};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;

#[derive(Debug)]
pub(super) struct HeadlessBackend {
    pub(super) app_name: String,
    pub(super) next_window_id: usize,
    pub(super) windows: BTreeMap<usize, WindowState>,
    // Retained to mirror native DefineStyle state; Phase 2b snapshots only expose applied style ids.
    pub(super) styles: HashMap<StyleId, ControlStyle>,
    pub(super) markers: Vec<String>,
    pub(super) dialog_requests: Vec<DialogRequest>,
    pub(super) dialog_responder: VecDeque<DialogScriptEntry>,
    pub(super) command_queue: VecDeque<PlatformCommand>,
    pub(super) follow_up_events: VecDeque<AppEvent>,
    pub(super) quitting: bool,
}

impl HeadlessBackend {
    pub(super) fn new(app_name: String) -> Self {
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

    pub(super) fn snapshot(&self) -> HeadlessSnapshot {
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

    pub(super) fn protocol_snapshot(&self) -> ProtocolSnapshot {
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

    pub(super) fn create_window(&mut self, config: WindowConfig<'_>) -> WindowId {
        let window_id = WindowId::new(self.next_window_id);
        self.next_window_id += 1;
        self.windows.insert(
            window_id.raw(),
            WindowState::new(window_id, config.title, config.width, config.height),
        );
        window_id
    }

    pub(super) fn execute_platform_command(
        &mut self,
        command: PlatformCommand,
    ) -> PlatformResult<()> {
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

    pub(super) fn handle_save_file_dialog(
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

    pub(super) fn handle_open_file_dialog(
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

    pub(super) fn handle_profile_selection_dialog(
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

    pub(super) fn handle_input_dialog(
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

    pub(super) fn handle_exclude_patterns_dialog(
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

    pub(super) fn handle_form_dialog(
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

    pub(super) fn handle_message_box(
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

    pub(super) fn handle_folder_picker_dialog(
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

    pub(super) fn record_dialog_request(&mut self, request: DialogRequest) {
        self.dialog_requests.push(request);
    }

    pub(super) fn complete_dialog_request(&mut self, request: DialogRequest) {
        self.record_dialog_request(request.clone());
        let outcome = self.take_dialog_outcome_for(&request);
        if let Some(event) = self.dialog_completion_event(&request, outcome) {
            self.follow_up_events.push_back(event);
        }
    }

    pub(super) fn take_dialog_outcome_for(&mut self, request: &DialogRequest) -> DialogOutcome {
        if let Some(front) = self.dialog_responder.front()
            && front.matcher.matches(request)
        {
            return self.dialog_responder.pop_front().unwrap().outcome;
        }
        Self::default_dialog_outcome(request)
    }

    pub(super) fn default_dialog_outcome(request: &DialogRequest) -> DialogOutcome {
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

    pub(super) fn dialog_completion_event(
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

    pub(super) fn unsupported_control_kind(command: &str, expected: &str) -> PlatformError {
        PlatformError::OperationFailed(format!(
            "{command} is only supported for {expected} controls"
        ))
    }

    pub(super) fn define_layout(
        &mut self,
        window_id: WindowId,
        rules: Vec<LayoutRule>,
    ) -> PlatformResult<()> {
        crate::contracts::validate_layout_rules(&rules)?;
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

    pub(super) fn populate_list_box(
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

    pub(super) fn set_list_box_selection(
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

    pub(super) fn set_combo_box_items(
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

    pub(super) fn set_combo_box_selection(
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

    pub(super) fn set_tab_bar_items(
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

    pub(super) fn set_tab_bar_selection(
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

    pub(super) fn set_check_box_checked(
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

    pub(super) fn set_radio_button_checked(
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

    pub(super) fn set_toggle_switch_state(
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

    pub(super) fn populate_tree_view(
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

    pub(super) fn update_tree_item_visual_state(
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

    pub(super) fn update_tree_item_text(
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

    pub(super) fn set_tree_view_selection(
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

    pub(super) fn expand_tree_items(
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

    pub(super) fn redraw_tree_item(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: TreeItemId,
    ) -> PlatformResult<()> {
        self.find_tree_item(window_id, control_id, item_id)?;
        Ok(())
    }

    pub(super) fn create_main_menu(
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

    pub(super) fn set_chart_data(
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

    pub(super) fn set_tab_bar_style(
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

    pub(super) fn define_style(
        &mut self,
        style_id: StyleId,
        style: ControlStyle,
    ) -> PlatformResult<()> {
        self.styles.insert(style_id, style);
        Ok(())
    }

    pub(super) fn apply_style_to_control(
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

    pub(super) fn set_toggle_switch_style(
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

    pub(super) fn set_scroll_position(
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

    pub(super) fn set_list_box_scroll_position(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        position: u32,
    ) -> PlatformResult<()> {
        self.validate_visible_enabled_list_box(window_id, control_id)?;
        self.with_control_mut(window_id, control_id, |control| {
            control.scroll_vertical = position;
            Ok(())
        })
    }

    pub(super) fn click_menu_action(
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

    pub(super) fn validate_visible_enabled_tree_view(
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

    pub(super) fn validate_visible_enabled_scrollable(
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

    pub(super) fn validate_visible_enabled_list_box(
        &self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<()> {
        self.validate_visible_enabled_control(window_id, control_id, "listbox action")?;
        let control = self.with_control_ref(window_id, control_id)?;
        match &control.kind {
            ControlKind::ListBox { .. } => Ok(()),
            _ => Err(Self::unsupported_control_kind("listbox action", "ListBox")),
        }
    }

    pub(super) fn validate_visible_enabled_input_keydown(
        &self,
        window_id: WindowId,
        control_id: ControlId,
    ) -> PlatformResult<()> {
        self.validate_visible_enabled_control(window_id, control_id, "key_input")?;
        let control = self.with_control_ref(window_id, control_id)?;
        match &control.kind {
            ControlKind::Input { .. } => Ok(()),
            _ => Err(Self::unsupported_control_kind("key_input", "Input")),
        }
    }

    pub(super) fn validate_window_visible(&self, window_id: WindowId) -> PlatformResult<()> {
        let window = self.window(window_id)?;
        if window.closed || !window.shown {
            return Err(PlatformError::InvalidHandle(format!(
                "WindowId {window_id:?} is not visible for menu action"
            )));
        }
        Ok(())
    }

    pub(super) fn find_tree_item(
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

    pub(super) fn toggle_tree_item(
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

    pub(super) fn set_rich_edit_content(
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

    pub(super) fn update_label_text(
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

    pub(super) fn set_control_text(
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

    pub(super) fn set_input_text(
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

    pub(super) fn select_list_box_item(
        &mut self,
        window_id: WindowId,
        control_id: ControlId,
        item_id: ListBoxItemId,
    ) -> PlatformResult<()> {
        self.validate_visible_enabled_control(window_id, control_id, "select_row")?;
        self.set_list_box_selection(window_id, control_id, item_id)
    }

    pub(super) fn toggle_control(
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

    pub(super) fn select_radio_button(
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

    pub(super) fn clear_other_radio_buttons(
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

    pub(super) fn validate_visible_enabled_control(
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

    pub(super) fn validate_visible_enabled_input(
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

    pub(super) fn validate_visible_enabled_combo(
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

    pub(super) fn validate_visible_enabled_tab(
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

    pub(super) fn validate_visible_enabled_button(
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

    pub(super) fn create_control(
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

    pub(super) fn with_window_mut<T>(
        &mut self,
        window_id: WindowId,
        f: impl FnOnce(&mut WindowState) -> PlatformResult<T>,
    ) -> PlatformResult<T> {
        let window = self.windows.get_mut(&window_id.raw()).ok_or_else(|| {
            PlatformError::InvalidHandle(format!("WindowId {window_id:?} not found"))
        })?;
        f(window)
    }

    pub(super) fn with_window_and_control_mut<T>(
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

    pub(super) fn with_control_mut<T>(
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

    pub(super) fn with_control_ref(
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

    pub(super) fn ensure_window_exists(&self, window_id: WindowId) -> PlatformResult<()> {
        self.window(window_id).map(|_| ())
    }

    pub(super) fn window(&self, window_id: WindowId) -> PlatformResult<&WindowState> {
        self.windows.get(&window_id.raw()).ok_or_else(|| {
            PlatformError::InvalidHandle(format!("WindowId {window_id:?} not found"))
        })
    }

    pub(super) fn window_mut(&mut self, window_id: WindowId) -> PlatformResult<&mut WindowState> {
        self.windows.get_mut(&window_id.raw()).ok_or_else(|| {
            PlatformError::InvalidHandle(format!("WindowId {window_id:?} not found"))
        })
    }
}
