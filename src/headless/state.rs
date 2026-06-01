use super::snapshot::{ControlSnapshot, LayoutRuleSnapshot, MenuNodeSnapshot, WindowSnapshot};
use crate::{
    ChartDataPacket, CheckState, ControlId, LabelClass, LayoutRule, ListBoxItemDescriptor,
    ListBoxItemId, ListBoxRowDensity, MessageSeverity, PlatformError, PlatformResult,
    SplitterOrientation, TreeItemDescriptor, TreeItemId, WindowId,
};
use std::collections::BTreeMap;

#[derive(Debug)]
pub(super) struct WindowState {
    pub(super) window_id: WindowId,
    pub(super) title: String,
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) shown: bool,
    pub(super) closed: bool,
    pub(super) focused_control_id: Option<ControlId>,
    pub(super) menu_items: Vec<MenuNode>,
    pub(super) layout_rules: Vec<LayoutRule>,
    pub(super) controls: BTreeMap<i32, ControlState>,
    pub(super) next_control_order: usize,
}

impl WindowState {
    pub(super) fn new(window_id: WindowId, title: &str, width: i32, height: i32) -> Self {
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

    pub(super) fn ensure_open(&self) -> PlatformResult<()> {
        if self.closed {
            return Err(PlatformError::InvalidHandle(format!(
                "WindowId {:?} is closed",
                self.window_id
            )));
        }
        Ok(())
    }

    pub(super) fn ensure_not_closed(&self) -> PlatformResult<()> {
        if self.closed {
            return Err(PlatformError::InvalidHandle(format!(
                "WindowId {:?} is closed",
                self.window_id
            )));
        }
        Ok(())
    }

    pub(super) fn snapshot(&self) -> WindowSnapshot {
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
pub(super) struct ControlState {
    pub(super) control_id: ControlId,
    pub(super) parent_control_id: Option<ControlId>,
    pub(super) creation_order: usize,
    pub(super) enabled: bool,
    pub(super) selected_all: bool,
    // Reserved for Phase 2 surface area; Phase 1 only records logical state and focus.
    pub(super) scroll_vertical: u32,
    // Reserved for Phase 2 surface area; Phase 1 only records logical state and focus.
    pub(super) scroll_horizontal: u32,
    // Reserved for Phase 2 styling/application commands.
    pub(super) style_id: Option<String>,
    pub(super) kind: ControlKind,
}

#[derive(Debug)]
pub(super) enum ControlKind {
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
pub(super) struct TreeItemNode {
    pub(super) id: TreeItemId,
    pub(super) text: String,
    pub(super) is_folder: bool,
    pub(super) state: CheckState,
    pub(super) expanded: bool,
    pub(super) style_override: Option<String>,
    pub(super) children: Vec<TreeItemNode>,
}

#[derive(Debug)]
pub(super) struct ChartDataState {
    pub(super) lines: Vec<ChartLineState>,
    pub(super) week_labels: Vec<String>,
    pub(super) is_loading: bool,
    pub(super) show_x_axis_labels: bool,
    pub(super) show_y_axis_labels: bool,
    pub(super) show_end_labels: bool,
}

#[derive(Debug)]
pub(super) struct ChartLineState {
    pub(super) label: String,
    pub(super) weekly_counts: Vec<u32>,
    pub(super) end_label: Option<String>,
    pub(super) emphasis: String,
}

#[derive(Debug)]
pub(super) struct MenuNode {
    pub(super) action: Option<u32>,
    pub(super) text: String,
    pub(super) children: Vec<MenuNode>,
}

impl TreeItemNode {
    pub(super) fn from_descriptor(descriptor: &TreeItemDescriptor) -> Self {
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

    pub(super) fn expand_recursive(&mut self) {
        self.expanded = true;
        for child in &mut self.children {
            child.expand_recursive();
        }
    }
}

impl ChartDataState {
    pub(super) fn from_packet(packet: ChartDataPacket) -> Self {
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
    pub(super) fn from_line(line: crate::ChartLineData) -> Self {
        Self {
            label: line.label,
            weekly_counts: line.weekly_counts,
            end_label: line.end_label,
            emphasis: line.emphasis.stable_name().to_string(),
        }
    }
}

impl MenuNode {
    pub(super) fn from_config(config: &crate::MenuItemConfig) -> Self {
        Self {
            action: config.action.map(|action| action.raw()),
            text: config.text.clone(),
            children: config.children.iter().map(MenuNode::from_config).collect(),
        }
    }
}

pub(super) fn tree_items_find(
    items: &[TreeItemNode],
    item_id: TreeItemId,
) -> Option<&TreeItemNode> {
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

pub(super) fn tree_items_find_mut(
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

pub(super) fn tree_items_contain_id(items: &[TreeItemNode], item_id: TreeItemId) -> bool {
    tree_items_find(items, item_id).is_some()
}

pub(super) fn find_menu_action(items: &[MenuNode], action_id: u32) -> Option<&MenuNode> {
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
