use super::state::{
    ChartDataState, ChartLineState, ControlKind, ControlState, MenuNode, TreeItemNode,
};
use super::{DialogRequest, DialogRequestDetails};
use crate::{BadgeDescriptor, LayoutRule, ListBoxItemDescriptor};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
pub(super) struct HeadlessSnapshot {
    pub(super) app_name: String,
    pub(super) quitting: bool,
    pub(super) markers: Vec<String>,
    pub(super) dialog_requests: Vec<DialogRequestSnapshot>,
    pub(super) windows: Vec<WindowSnapshot>,
}

#[derive(Debug, Serialize)]
pub(super) struct ProtocolSnapshot {
    pub(super) app_name: String,
    pub(super) dialog_requests: Vec<DialogRequestSnapshot>,
    pub(super) windows: Vec<WindowSnapshot>,
}

#[derive(Debug, Serialize)]
pub(super) struct WindowSnapshot {
    pub(super) id: usize,
    pub(super) title: String,
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) shown: bool,
    pub(super) closed: bool,
    pub(super) focused_control_id: Option<i32>,
    pub(super) menu: Vec<MenuNodeSnapshot>,
    pub(super) layout_rules: Vec<LayoutRuleSnapshot>,
    pub(super) controls: Vec<ControlSnapshot>,
}

#[derive(Debug, Serialize)]
pub(super) struct LayoutRuleSnapshot {
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
pub(super) struct MenuNodeSnapshot {
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
pub(super) enum ControlSnapshot {
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
pub(super) struct TreeItemSnapshot {
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
pub(super) struct ChartDataSnapshot {
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
pub(super) struct ChartLineSnapshot {
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
pub(super) struct DialogRequestSnapshot {
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
pub(super) enum DialogRequestDetailsSnapshot {
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

#[derive(Debug, Serialize)]
pub(super) struct FormDialogSnapshot {
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
pub(super) enum FormRowSnapshot {
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
pub(super) enum FormFieldSnapshot {
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
pub(super) struct FormFileExistsWarningSnapshot {
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
pub(super) struct FormButtonsSnapshot {
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
pub(super) struct ListBoxItemSnapshot {
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
pub(super) struct BadgeSnapshot {
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
