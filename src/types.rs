/*
 * This module defines core data types used for communication between the
 * application logic and the platform layer. It includes identifiers for windows
 * and tree items, configurations for UI elements (windows, menus),
 * platform-agnostic event types (`AppEvent`), commands for the platform layer
 * (`PlatformCommand`), severity levels for messages (`MessageSeverity`),
 * and semantic identifiers for menu actions (`MenuActionId`). It also defines the
 * `PlatformEventHandler` trait that the application logic must implement.
 */

use std::path::PathBuf;

use super::styling_primitives::{ControlStyle, StyleId};

// An opaque identifier for a native window, managed by the platform layer.
//
// The application logic layer uses this ID to refer to specific windows
// when sending commands or receiving events, without needing to know about
// native window handles like HWND.
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

// An opaque identifier for an item within a tree-like control (e.g., TreeView).
//
// This ID is generated and managed by the application logic layer and used to
// uniquely identify tree items in commands and events. The platform layer
// maps this to native tree item handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TreeItemId(pub u64);

/*
 * Represents a logical UI control identifier shared between the app logic
 * and the platform layer. Wrapping the raw `i32` ID prevents accidental
 * mixing with unrelated integer values and documents the intended usage.
 * The platform layer maps these logical IDs to native control handles.
 * [CDU-ControlLogicalIdsV1] Every control is referenced through this strongly typed wrapper instead of raw HWND values.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlId(pub i32);

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

/*
 * Strongly typed identifier for application-defined menu actions.
 *
 * Wrapping the raw `u32` in a dedicated newtype keeps the public API generic
 * while still allowing consumers to use compile-time constants for their menu
 * semantics. [CDU-MenuActionIdV1] The platform layer only needs to track these
 * opaque IDs and map them to Win32 menu command identifiers.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MenuActionId(pub u32);

// --- Data Structures for UI Description (Platform-Agnostic) ---

// Configuration for creating a new native window.
//
// Provided by the application logic to the platform layer, describing
// the desired properties of a window without specifying native details.
#[derive(Debug, Clone)]
pub struct WindowConfig<'a> {
    pub title: &'a str,
    pub width: i32,
    pub height: i32,
}

// Represents the visual check state of an item, typically a checkbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Checked,
    Unchecked,
}

// Describes a single item to be displayed in a tree-like control.
//
// This structure is used by the application logic to define the content
// and hierarchy of a tree view, which the platform layer then renders.
#[derive(Debug, Clone)]
pub struct TreeItemDescriptor {
    pub id: TreeItemId,
    pub text: String,
    pub is_folder: bool,
    pub state: CheckState,
    pub children: Vec<TreeItemDescriptor>,
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

/*
 * Configuration for a single menu item, used by `PlatformCommand::CreateMainMenu`.
 *
 * Each item can opt into semantic routing by providing a `MenuActionId`; popups
 * omit the action and supply child `MenuItemConfig` entries instead.
 */
#[derive(Debug, Clone)]
pub struct MenuItemConfig {
    pub action: Option<MenuActionId>,
    pub text: String,
    pub children: Vec<MenuItemConfig>, // For submenus
}

// --- Layout Primitives ---

/*
 * Defines how a control should dock within its parent container.
 * This is a simplified docking model. More advanced anchoring or grid systems
 * could be introduced later.
 * TODO: I think many of these have not been implemented yet.
 */
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DockStyle {
    None,   // No docking, control is positioned manually or by other rules.
    Top,    // Docks to the top edge of the container.
    Bottom, // Docks to the bottom edge of the container.
    Left,   // Docks to the left edge of the container.
    Right,  // Docks to the right edge of the container.
    Fill,   // Fills all remaining space in the container (both axes).
    ProportionalFill { weight: f32 }, // Fills space along main axis proportionally with siblings.
}

/*
 * A rule that associates a control (by its ID) with a specific docking style.
 * The `order` field can be used to determine the sequence in which docking
 * calculations are performed (e.g., top/bottom docks first, then left/right, then fill).
 * Lower order values are typically processed first.
 * The `parent_control_id` specifies the logical ID of the parent control; `None` indicates
 * the main window client area is the parent.
 */
#[derive(Debug, Clone)]
pub struct LayoutRule {
    pub control_id: ControlId, // The ID of the control this rule applies to.
    pub parent_control_id: Option<ControlId>, // ID of the parent control, None for main window.
    pub dock_style: DockStyle,
    pub order: u32, // Order of application (e.g., 0 for top, 1 for bottom, 10 for fill)
    pub fixed_size: Option<i32>, // For Top/Bottom, this is height. For Left/Right, this is width. Not used for Fill/None.
    pub margin: (i32, i32, i32, i32), // (top, right, bottom, left) margins around the control.
}

// --- Events from Platform to App Logic ---

/*
 * Represents platform-agnostic UI events generated by the native toolkit.
 *
 * The platform layer translates native OS events into these types and
 * sends them to the application logic layer for handling. Menu item clicks
 * are now generalized into `MenuActionClicked`.
 *
 * ## Splitter Event Contract
 *
 * Splitter controls (when implemented) emit events with `desired_left_width_px` in
 * **window client coordinates** (not raw mouse x). The platform layer is responsible
 * for computing the desired width by:
 * 1. Converting the mouse position to window client coordinates
 * 2. Accounting for any relevant margins
 * 3. Accounting for the splitter's own thickness/width
 *
 * This ensures the application logic receives a value that directly represents
 * the intended total width of the left region, ready for clamping and state updates.
 */
#[derive(Debug)]
pub enum AppEvent {
    WindowCloseRequestedByUser {
        window_id: WindowId,
    },
    // Signals that a window has been resized.
    WindowResized {
        window_id: WindowId,
        width: i32,
        height: i32,
    },
    // Signals that a window and its native resources have been destroyed.
    // The `WindowId` should be considered invalid after this event.
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
    // Signals that a button was clicked.
    ButtonClicked {
        window_id: WindowId,
        control_id: ControlId,
    },
    // Signals that a menu item was clicked, identified by its semantic `MenuActionId`.
    MenuActionClicked {
        action_id: MenuActionId,
    },
    // Signals the result of a "Save File" dialog.
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
    ExcludePatternsDialogCompleted {
        window_id: WindowId,
        saved: bool,
        patterns: String,
    },
    FolderPickerDialogCompleted {
        window_id: WindowId,
        path: Option<PathBuf>,
    },
    // Signals that the initial static UI setup for the main window is complete.
    MainWindowUISetupComplete {
        window_id: WindowId,
    },
    ControlScrolled {
        window_id: WindowId,
        control_id: ControlId,
        vertical_pos: u32,
        horizontal_pos: u32,
    },
    // Signals that text was entered in an input control after debouncing.
    InputTextChanged {
        window_id: WindowId,
        control_id: ControlId,
        text: String,
    },
    // Signals that a splitter is being dragged by the user.
    // Emitted continuously during the drag operation.
    // The `desired_left_width_px` is in window client coordinates and represents
    // the total width the left region should have (see Splitter Event Contract above).
    SplitterDragging {
        window_id: WindowId,
        control_id: ControlId,
        desired_left_width_px: i32,
    },
    // Signals that the user has finished dragging the splitter (mouse button released).
    // The `desired_left_width_px` is the final position in window client coordinates.
    SplitterDragEnded {
        window_id: WindowId,
        control_id: ControlId,
        desired_left_width_px: i32,
    },
    // Signals that a ComboBox selection was changed by the user.
    // None means no selection (CB_ERR).
    ComboBoxSelectionChanged {
        window_id: WindowId,
        control_id: ControlId,
        selected_index: Option<usize>,
    },
    // Signals that a RadioButton was selected by the user.
    RadioButtonSelected {
        window_id: WindowId,
        control_id: ControlId,
    },
}

// Defines the severity of a message to be displayed, e.g., in the status bar.
// Ordered from least to most severe for comparison. `None` clears.
// TODO: 'None' isn't used, is it needed?
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessageSeverity {
    None,        // Clears the status, or lowest priority if not explicitly clearing
    Information, // Neutral information
    Warning,     // A warning to the user
    Error,       // An error has occurred
}

// --- Label Classification ---
// TODO: Only 'StatusBar' is currently used, is it needed?
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelClass {
    Default,
    StatusBar,
}

// --- Splitter Orientation ---

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

// Represents platform-agnostic commands sent from the application logic to the platform layer.
//
// These commands instruct the platform layer to perform specific actions on
// native UI elements.
// TODO: All commands that create controls should use the same name for this ID. E.g. "control_id".
#[allow(dead_code)]
#[derive(Debug, Clone)]
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
    /*
     * Shows a simple modal message box from the platform layer.
     * The application logic provides the window owner, title, body text,
     * and a severity that the platform layer maps to the appropriate icon.
     */
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
    // Signals to the platform layer that all initial UI description commands
    // for the main window have been enqueued and processed.
    SignalMainWindowUISetupComplete {
        window_id: WindowId,
    },
    // New command to define layout rules for controls within a window.
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
    // TODO: Now that 'CreateInput' can be used to create a read-only item, it should probably change name.
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
    UpdateLabelText {
        window_id: WindowId,
        control_id: ControlId,
        text: String,
        severity: MessageSeverity,
    },
    // Expands only the currently visible items in a TreeView. Used when a filter is active.
    ExpandVisibleTreeItems {
        window_id: WindowId,
        control_id: ControlId,
    },
    // Expands all items in a TreeView regardless of visibility.
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

    // --- Style Management Commands ---
    // This style can then be applied to controls.
    DefineStyle {
        style_id: StyleId,
        style: ControlStyle,
    },
    // Applies a previously defined style to a specific control.
    ApplyStyleToControl {
        window_id: WindowId,
        control_id: ControlId,
        style_id: StyleId,
    },
}

// --- Trait for App Logic to Handle Events ---

// A trait to be implemented by the application logic layer to handle UI events.
//
// The platform layer calls methods on this trait to notify the application
// logic about user interactions or system events.
pub trait PlatformEventHandler: Send + Sync + 'static {
    // Called by the platform layer when a native UI event has been processed.
    // The implementor should handle the event and enqueue `PlatformCommand`s
    // for the platform layer to execute.
    fn handle_event(&mut self, event: AppEvent);

    // Called by the platform layer when the application is about to exit its main loop.
    // This allows the application logic to perform any necessary cleanup.
    fn on_quit(&mut self) {}

    // Attempts to dequeue a single `PlatformCommand` from the internal queue.
    // This is called by the platform layer's run loop.
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
