//! Core platform-agnostic types shared between host logic and the Win32 backend.

use std::path::PathBuf;

use super::styling_primitives::{Color, ControlStyle, FontDescription, StyleId};

/// Opaque identifier for a native top-level window managed by the platform layer.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub(crate) usize);

impl WindowId {
    pub const fn new(raw: usize) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> usize {
        self.0
    }
}

/// Opaque identifier for an item within a tree-like control.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TreeItemId(pub(crate) u64);

impl TreeItemId {
    /// Creates a tree-item identifier from the host-defined raw value.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the host-defined raw value.
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// An opaque identifier for an item in the custom list control.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListBoxItemId(pub(crate) u64);

impl ListBoxItemId {
    /// Creates a list-box item identifier from the host-defined raw value.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the host-defined raw value.
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Badge payload rendered inside a custom list-row pill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadgeDescriptor {
    pub text: String,
    pub style: StyleId,
}

/// Describes a structured row rendered by the custom list control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListBoxItemDescriptor {
    pub id: ListBoxItemId,
    pub badges: Vec<BadgeDescriptor>,
    pub title: String,
    pub metadata: String,
    pub enabled: bool,
}

/// Controls whether a list box renders standard two-line rows or a compact title-only density.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListBoxRowDensity {
    Expanded,
    Compact,
}

/// Logical identifier for a control managed by the platform layer.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlId(pub(crate) i32);

impl ControlId {
    pub const fn new(raw: i32) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> i32 {
        self.0
    }
}

impl From<i32> for ControlId {
    fn from(raw: i32) -> Self {
        Self(raw)
    }
}

impl From<ControlId> for i32 {
    fn from(control_id: ControlId) -> Self {
        control_id.0
    }
}

/// Strongly typed identifier for an application-defined menu action.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MenuActionId(pub(crate) u32);

impl MenuActionId {
    /// Creates a menu action identifier from a host-defined raw value.
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the host-defined raw value.
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl From<u32> for MenuActionId {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

impl From<MenuActionId> for u32 {
    fn from(action_id: MenuActionId) -> Self {
        action_id.0
    }
}

/// Configuration for creating a new top-level native window.
#[derive(Debug, Clone)]
pub struct WindowConfig<'a> {
    /// Initial window title.
    pub title: &'a str,
    /// Initial client width in pixels.
    pub width: i32,
    /// Initial client height in pixels.
    pub height: i32,
}

/// Visual state-image lane of an item in a tree control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    /// Reserves the lane but leaves it blank.
    Hidden,
    /// Shows a checked state image.
    Checked,
    /// Shows an unchecked state image.
    Unchecked,
}

/// Descriptor for one tree item, including its children and style override.
#[derive(Debug, Clone)]
pub struct TreeItemDescriptor {
    /// Stable host-defined identifier for this item.
    pub id: TreeItemId,
    /// Visible label text.
    pub text: String,
    /// Whether the item should be rendered with folder affordances.
    pub is_folder: bool,
    /// Requested checkbox/state-image lane state.
    pub state: CheckState,
    /// Nested child items.
    pub children: Vec<TreeItemDescriptor>,
    /// Optional semantic style override.
    pub style_override: Option<StyleId>,
}

/// Identifies the optional color marker that can be rendered next to a tree item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeItemMarkerKind {
    None,
    Blue,
    Green,
    Yellow,
    Red,
    Purple,
    Gray,
}

/// Configuration for one menu item in a declarative menu tree.
#[derive(Debug, Clone)]
pub struct MenuItemConfig {
    /// Action routed back through [`AppEvent::MenuActionClicked`], if any.
    pub action: Option<MenuActionId>,
    /// Menu caption text, including Win32 accelerator markers such as `&File`.
    pub text: String,
    /// Child items for popup menus.
    pub children: Vec<MenuItemConfig>,
}

/// Docking rule used by the built-in layout engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DockStyle {
    /// No docking. Positioning is handled elsewhere.
    None,
    /// Dock to the top edge of the parent container.
    Top,
    /// Dock to the bottom edge of the parent container.
    Bottom,
    /// Dock to the left edge of the parent container.
    Left,
    /// Dock to the right edge of the parent container.
    Right,
    /// Fill the remaining space in the parent container.
    Fill,
    /// Fill space along the main axis proportionally with siblings.
    ProportionalFill { weight: f32 },
}

/// Associates a control with a docking style inside a parent container.
#[derive(Debug, Clone)]
pub struct LayoutRule {
    /// Control that the rule applies to.
    pub control_id: ControlId,
    /// Parent control, or `None` for the main window client area.
    pub parent_control_id: Option<ControlId>,
    /// Docking behavior to apply.
    pub dock_style: DockStyle,
    /// Application order for sibling layout rules.
    pub order: u32,
    /// Fixed width or height for edge docking modes.
    pub fixed_size: Option<i32>,
    /// Margins in `(top, right, bottom, left)` order.
    pub margin: (i32, i32, i32, i32),
}

/// Platform-agnostic UI events translated from native toolkit activity.
#[derive(Debug)]
#[non_exhaustive]
pub enum AppEvent {
    WindowCloseRequestedByUser {
        window_id: WindowId,
    },
    WindowResized {
        window_id: WindowId,
        width: i32,
        height: i32,
    },
    /// Signals that a window resize drag has completed.
    /// Dimensions are outer window dimensions (including frame/title bar).
    WindowResizeCompleted {
        window_id: WindowId,
        outer_width: i32,
        outer_height: i32,
    },
    WindowDestroyed {
        window_id: WindowId,
    },
    TreeViewItemToggledByUser {
        window_id: WindowId,
        item_id: TreeItemId,
        new_state: CheckState,
    },
    TreeViewItemSelectionChanged {
        window_id: WindowId,
        item_id: TreeItemId,
    },
    ListBoxItemSelectionChanged {
        window_id: WindowId,
        control_id: ControlId,
        item_id: ListBoxItemId,
    },
    ListBoxItemKeyDown {
        window_id: WindowId,
        control_id: ControlId,
        key_code: u16,
    },
    ListBoxScrolled {
        window_id: WindowId,
        control_id: ControlId,
        position: u32,
    },
    ButtonClicked {
        window_id: WindowId,
        control_id: ControlId,
    },
    MenuActionClicked {
        action_id: MenuActionId,
    },
    FileSaveDialogCompleted {
        window_id: WindowId,
        result: Option<std::path::PathBuf>,
    },
    FileOpenProfileDialogCompleted {
        window_id: WindowId,
        result: Option<PathBuf>,
    },
    ProfileSelectionDialogCompleted {
        window_id: WindowId,
        chosen_profile_name: Option<String>,
        create_new_requested: bool,
        user_cancelled: bool,
    },
    GenericInputDialogCompleted {
        window_id: WindowId,
        text: Option<String>,
        context_tag: Option<String>,
    },
    FormDialogCompleted {
        window_id: WindowId,
        context_tag: String,
        confirmed: bool,
        field_values: Vec<FormFieldValue>,
    },
    ExcludePatternsDialogCompleted {
        window_id: WindowId,
        saved: bool,
        patterns: String,
    },
    FolderPickerDialogCompleted {
        window_id: WindowId,
        path: Option<PathBuf>,
    },
    MainWindowUISetupComplete {
        window_id: WindowId,
    },
    ControlScrolled {
        window_id: WindowId,
        control_id: ControlId,
        vertical_pos: u32,
        horizontal_pos: u32,
    },
    InputTextChanged {
        window_id: WindowId,
        control_id: ControlId,
        text: String,
    },
    SplitterDragging {
        window_id: WindowId,
        control_id: ControlId,
        desired_left_width_px: i32,
    },
    SplitterDragEnded {
        window_id: WindowId,
        control_id: ControlId,
        desired_left_width_px: i32,
    },
    ComboBoxSelectionChanged {
        window_id: WindowId,
        control_id: ControlId,
        selected_index: Option<usize>,
    },
    RadioButtonSelected {
        window_id: WindowId,
        control_id: ControlId,
    },
    CheckBoxToggled {
        window_id: WindowId,
        control_id: ControlId,
        checked: bool,
    },
    TabBarSelectionChanged {
        window_id: WindowId,
        control_id: ControlId,
        selected_index: usize,
    },
    ToggleSwitchToggled {
        window_id: WindowId,
        control_id: ControlId,
        checked: bool,
    },
}

/// Severity level used by status and message-box surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum MessageSeverity {
    /// Clears the status surface or represents the lowest priority.
    None,
    /// Neutral information.
    Information,
    /// Warning state.
    Warning,
    /// Error state.
    Error,
}

/// Semantic classification for label controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelClass {
    Default,
    StatusBar,
}

/// Defines the orientation of a splitter control.
///
/// - `Vertical`: Creates a vertical splitter bar that divides left/right regions.
///   The user drags the splitter horizontally to resize the left and right panels.
/// - `Horizontal`: Creates a horizontal splitter bar that divides top/bottom regions.
///   The user drags the splitter vertically to resize the top and bottom panels.
///   (Reserved for future implementation)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitterOrientation {
    Vertical,   // Divides left/right (user drags horizontally)
    Horizontal, // Divides top/bottom (future extension)
}

/// Visual emphasis level for a chart line.
#[derive(Debug, Clone)]
pub enum ChartLineEmphasis {
    Primary,
    Secondary,
}

/// A single entity trend line sent to the chart control.
#[derive(Debug, Clone)]
pub struct ChartLineData {
    /// Entity name shown in the legend.
    pub label: String,
    /// Weekly mention counts (one per x-axis point).
    pub weekly_counts: Vec<u32>,
    /// COLORREF (0x00BBGGRR) for this line's pen.
    pub color: u32,
    /// Optional short label drawn at the right end of the line.
    pub end_label: Option<String>,
    /// Visual emphasis level for this line.
    pub emphasis: ChartLineEmphasis,
}

/// Full data payload for a `SetChartData` command.
#[derive(Debug, Clone)]
pub struct ChartDataPacket {
    /// Lines to draw (index 0 = top legend entry). Max 10.
    pub lines: Vec<ChartLineData>,
    /// X-axis week label strings (same length as each `weekly_counts`).
    pub week_labels: Vec<String>,
    /// When true the chart renders an empty "Loading…" state.
    pub is_loading: bool,
    /// When true the chart renders x-axis week labels below the plot area.
    pub show_x_axis_labels: bool,
    /// When true the chart renders y-axis tick values left of the plot area.
    pub show_y_axis_labels: bool,
    /// When true the chart renders an end label at the right of each line.
    pub show_end_labels: bool,
}

/// Generic rows that can appear in a modal form dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormRow {
    ReadOnlyText {
        label: String,
        value: String,
    },
    Note {
        text: String,
        severity: MessageSeverity,
    },
}

/// Validation rules for a single-line text input in a generic form dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormTextValidation {
    Any,
    NonEmpty,
    PathSegment,
}

/// Optional live warning attached to a text input field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormFileExistsWarning {
    pub base_dir: PathBuf,
    pub message: String,
}

/// Generic editable fields supported by the modal form dialog primitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormField {
    TextInput {
        field_id: String,
        label: String,
        value: String,
        validation: FormTextValidation,
        live_warning: Option<FormFileExistsWarning>,
    },
    CheckBox {
        field_id: String,
        label: String,
        checked: bool,
    },
}

/// Generic button labels and initial confirm-enabled state for a modal form dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormButtons {
    pub confirm_label: String,
    pub cancel_label: String,
    pub confirm_enabled: bool,
}

/// Descriptor for a generic modal form dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormDialogDescriptor {
    pub title: String,
    pub context_tag: String,
    pub rows: Vec<FormRow>,
    pub fields: Vec<FormField>,
    pub buttons: FormButtons,
}

/// Result payload returned by a generic modal form dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormFieldValue {
    Text { field_id: String, value: String },
    CheckBox { field_id: String, checked: bool },
}

/// Platform-agnostic commands sent from host logic to the platform layer.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PlatformCommand {
    SetWindowTitle {
        window_id: WindowId,
        title: String,
    },
    ShowWindow {
        window_id: WindowId,
    },
    CloseWindow {
        window_id: WindowId,
    },
    PopulateTreeView {
        window_id: WindowId,
        control_id: ControlId, /* New: Logical ID of the TreeView to populate */
        items: Vec<TreeItemDescriptor>,
    },
    UpdateTreeItemVisualState {
        window_id: WindowId,
        control_id: ControlId, /* New: Logical ID of the TreeView containing the item */
        item_id: TreeItemId,
        new_state: CheckState,
    },
    UpdateTreeItemText {
        window_id: WindowId,
        control_id: ControlId,
        item_id: TreeItemId,
        text: String,
    },
    ShowSaveFileDialog {
        window_id: WindowId,
        title: String,
        default_filename: String,
        filter_spec: String,
        initial_dir: Option<PathBuf>,
    },
    ShowOpenFileDialog {
        window_id: WindowId,
        title: String,
        filter_spec: String,
        initial_dir: Option<PathBuf>,
    },
    ShowProfileSelectionDialog {
        window_id: WindowId,
        available_profiles: Vec<String>,
        title: String,
        prompt: String,
    },
    ShowInputDialog {
        window_id: WindowId,
        title: String,
        prompt: String,
        default_text: Option<String>,
        context_tag: Option<String>,
    },
    ShowExcludePatternsDialog {
        window_id: WindowId,
        title: String,
        patterns: String,
    },
    ShowFormDialog {
        window_id: WindowId,
        form: FormDialogDescriptor,
    },
    ShowMessageBox {
        window_id: WindowId,
        title: String,
        message: String,
        severity: MessageSeverity,
    },
    ShowFolderPickerDialog {
        window_id: WindowId,
        title: String,
        initial_dir: Option<PathBuf>,
    },
    SetControlEnabled {
        window_id: WindowId,
        control_id: ControlId,
        enabled: bool,
    },
    QuitApplication,

    CreateMainMenu {
        window_id: WindowId,
        menu_items: Vec<MenuItemConfig>,
    },
    CreateButton {
        window_id: WindowId,
        parent_control_id: Option<ControlId>, // None means child of main window's client area
        control_id: ControlId, // The existing logical ID (e.g., ID_BUTTON_GENERATE_ARCHIVE)
        text: String,
        // Position/size will be managed by DefineLayout command.
    },
    CreateTreeView {
        window_id: WindowId,
        parent_control_id: Option<ControlId>, // The logical ID for the parent, None for main window
        control_id: ControlId,                // The logical ID for the TreeView
    },
    CreateListBox {
        window_id: WindowId,
        parent_control_id: Option<ControlId>,
        control_id: ControlId,
    },
    PopulateListBox {
        window_id: WindowId,
        control_id: ControlId,
        items: Vec<ListBoxItemDescriptor>,
        badge_column_width: u16,
    },
    SetListBoxRowDensity {
        window_id: WindowId,
        control_id: ControlId,
        density: ListBoxRowDensity,
    },
    SignalMainWindowUISetupComplete {
        window_id: WindowId,
    },
    DefineLayout {
        window_id: WindowId,
        rules: Vec<LayoutRule>,
    },
    CreatePanel {
        window_id: WindowId,
        parent_control_id: Option<ControlId>, // None means child of main window's client area
        control_id: ControlId,                // Logical ID for this new panel
    },
    CreateLabel {
        window_id: WindowId,
        parent_control_id: Option<ControlId>, // None means child of main window's client area
        control_id: ControlId,
        initial_text: String,
        class: LabelClass, // Classify labels for potential specific styling
    },
    CreateInput {
        window_id: WindowId,
        parent_control_id: Option<ControlId>,
        control_id: ControlId,
        initial_text: String,
        read_only: bool,
        multiline: bool,
        vertical_scroll: bool,
    },
    CreateRichEdit {
        window_id: WindowId,
        parent_control_id: Option<ControlId>,
        control_id: ControlId,
    },
    /// Creates a GDI line chart control as a child of `parent_control_id`.
    CreateChart {
        window_id: WindowId,
        parent_control_id: Option<ControlId>,
        control_id: ControlId,
    },
    /// Replaces the chart's displayed data and triggers a repaint.
    SetChartData {
        window_id: WindowId,
        control_id: ControlId,
        data: ChartDataPacket,
    },
    CreateProgressBar {
        window_id: WindowId,
        parent_control_id: Option<ControlId>,
        control_id: ControlId,
    },
    CreateSplitter {
        window_id: WindowId,
        parent_control_id: Option<ControlId>,
        control_id: ControlId,
        orientation: SplitterOrientation,
    },
    SetProgressBarRange {
        window_id: WindowId,
        control_id: ControlId,
        min: u32,
        max: u32,
    },
    SetProgressBarPosition {
        window_id: WindowId,
        control_id: ControlId,
        position: u32,
    },
    SetControlText {
        window_id: WindowId,
        control_id: ControlId,
        text: String,
    },
    SetInputText {
        window_id: WindowId,
        control_id: ControlId,
        text: String,
    },
    SetViewerContent {
        window_id: WindowId,
        control_id: ControlId,
        text: String,
    },
    SetRichEditContent {
        window_id: WindowId,
        control_id: ControlId,
        rtf_text: String,
    },
    SetScrollPosition {
        window_id: WindowId,
        control_id: ControlId,
        vertical_pos: u32,
        horizontal_pos: u32,
    },
    SetTreeViewSelection {
        window_id: WindowId,
        control_id: ControlId,
        item_id: TreeItemId,
    },
    SetListBoxSelection {
        window_id: WindowId,
        control_id: ControlId,
        item_id: ListBoxItemId,
    },
    UpdateLabelText {
        window_id: WindowId,
        control_id: ControlId,
        text: String,
        severity: MessageSeverity,
    },
    ExpandVisibleTreeItems {
        window_id: WindowId,
        control_id: ControlId,
    },
    ExpandAllTreeItems {
        window_id: WindowId,
        control_id: ControlId,
    },
    RedrawTreeItem {
        window_id: WindowId,
        control_id: ControlId, /* New: Logical ID of the TreeView containing the item */
        item_id: TreeItemId,
    },
    CreateComboBox {
        window_id: WindowId,
        parent_control_id: Option<ControlId>,
        control_id: ControlId,
    },
    SetComboBoxItems {
        window_id: WindowId,
        control_id: ControlId,
        items: Vec<String>,
    },
    SetComboBoxSelection {
        window_id: WindowId,
        control_id: ControlId,
        selected_index: Option<usize>,
    },
    CreateRadioButton {
        window_id: WindowId,
        parent_control_id: Option<ControlId>,
        control_id: ControlId,
        text: String,
        group_start: bool,
    },
    SetRadioButtonChecked {
        window_id: WindowId,
        control_id: ControlId,
        checked: bool,
    },
    CreateCheckBox {
        window_id: WindowId,
        parent_control_id: Option<ControlId>,
        control_id: ControlId,
        text: String,
    },
    SetCheckBoxChecked {
        window_id: WindowId,
        control_id: ControlId,
        checked: bool,
    },
    /// Creates a custom TabBar control (bottom-accent-line style).
    CreateTabBar {
        window_id: WindowId,
        control_id: ControlId,
        parent_control_id: Option<ControlId>,
        items: Vec<String>,
    },
    /// Replaces all tab labels and triggers a repaint.
    SetTabBarItems {
        window_id: WindowId,
        control_id: ControlId,
        items: Vec<String>,
    },
    /// Drives the active tab from the reducer (no event emitted for programmatic changes).
    SetTabBarSelection {
        window_id: WindowId,
        control_id: ControlId,
        selected_index: usize,
    },
    /// Pushes resolved palette data from `StyleId::TabBar`/`TabBarAccent` into the control.
    SetTabBarStyle {
        window_id: WindowId,
        control_id: ControlId,
        background_color: Color,
        text_color: Color,
        accent_color: Color,
        font: Option<FontDescription>,
    },
    DefineStyle {
        style_id: StyleId,
        style: ControlStyle,
    },
    ApplyStyleToControl {
        window_id: WindowId,
        control_id: ControlId,
        style_id: StyleId,
    },
    /// Creates a custom sliding toggle switch control (pill + knob, fully owner-drawn).
    CreateToggleSwitch {
        window_id: WindowId,
        parent_control_id: Option<ControlId>,
        control_id: ControlId,
        label: String,
        checked: bool,
    },
    /// Programmatically sets the checked state of a toggle switch and repaints it.
    SetToggleSwitchState {
        window_id: WindowId,
        control_id: ControlId,
        checked: bool,
    },
    /// Pushes resolved palette colors into a toggle switch control.
    SetToggleSwitchStyle {
        window_id: WindowId,
        control_id: ControlId,
        background: Color,
        pill_off: Color,
        pill_on: Color,
        knob: Color,
        text: Color,
    },
}

// --- Trait for App Logic to Handle Events ---

/// Receives platform-agnostic UI events from the native platform layer.
///
/// # Threading and re-entrancy
///
/// `handle_event` runs synchronously on the UI thread while native message
/// dispatch is in progress. In the current Windows implementation this happens
/// inline from `Win32ApiInternalState::send_event`, so handlers are expected
/// not to block: blocking stalls native message dispatch and can hang the UI.
///
/// Handlers may enqueue `PlatformCommand` values for the platform layer to
/// execute on the next command-drain pass. Synchronous re-entrancy back into
/// the platform layer is unsupported.
pub trait PlatformEventHandler: Send + Sync + 'static {
    /// Handles one translated native event.
    fn handle_event(&mut self, event: AppEvent);

    /// Called before the platform layer exits its main loop.
    fn on_quit(&mut self) {}

    /// Attempts to dequeue one `PlatformCommand` for the platform run loop.
    fn try_dequeue_command(&mut self) -> Option<PlatformCommand>;
}

/// Provides synchronous access to UI state needed by the platform layer.
///
/// This trait allows the platform layer to query the application logic for
/// specific pieces of information without sending events. Currently it only
/// exposes the ability to check if a tree item should be drawn with the "new"
/// indicator and to request the marker that should be painted for that item.
pub trait UiStateProvider: Send + Sync + 'static {
    /// Queries if a specific tree item is currently in the "New" state.
    /// The platform layer uses this during custom drawing to determine if the
    /// "New" visual indicator (e.g., a blue circle) should be rendered for the
    /// item.
    fn is_tree_item_new(&self, window_id: WindowId, item_id: TreeItemId) -> bool;

    /// Asks the provider for the marker that should be drawn beside the tree item.
    fn tree_item_marker(&self, _window_id: WindowId, _item_id: TreeItemId) -> TreeItemMarkerKind {
        TreeItemMarkerKind::None
    }
}

#[cfg(test)]
mod tests {
    use super::{TreeItemId, TreeItemMarkerKind, UiStateProvider, WindowId};

    struct SilentProvider;

    impl UiStateProvider for SilentProvider {
        fn is_tree_item_new(&self, _window_id: WindowId, _item_id: TreeItemId) -> bool {
            false
        }
    }

    #[test]
    fn default_provider_returns_none() {
        let provider = SilentProvider;
        assert_eq!(
            provider.tree_item_marker(WindowId::new(1), TreeItemId(7)),
            TreeItemMarkerKind::None
        );
    }

    struct CustomMarkerProvider;

    impl UiStateProvider for CustomMarkerProvider {
        fn is_tree_item_new(&self, _window_id: WindowId, _item_id: TreeItemId) -> bool {
            false
        }

        fn tree_item_marker(
            &self,
            _window_id: WindowId,
            item_id: TreeItemId,
        ) -> TreeItemMarkerKind {
            if item_id.0 == 13 {
                TreeItemMarkerKind::Green
            } else {
                TreeItemMarkerKind::Purple
            }
        }
    }

    #[test]
    fn custom_provider_returns_expected_marker() {
        let provider = CustomMarkerProvider;
        assert_eq!(
            provider.tree_item_marker(WindowId::new(2), TreeItemId(13)),
            TreeItemMarkerKind::Green
        );
        assert_eq!(
            provider.tree_item_marker(WindowId::new(2), TreeItemId(99)),
            TreeItemMarkerKind::Purple
        );
    }
}
