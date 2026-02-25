/*
 * This module provides common Win32 windowing functionalities, including
 * window class registration, native window creation, and the main window
 * procedure (WndProc) for message handling. It defines `NativeWindowData`
 * to store per-window native state and helper functions for interacting
 * with the Win32 API.
 *
 * For control-specific message handling (e.g., TreeView notifications,
 * label custom drawing), this module now primarily dispatches to dedicated
 * handlers in the `super::controls` module.
 */
use super::{
    app::Win32ApiInternalState,
    controls::{
        button_handler, checkbox_handler, combobox_handler, input_handler, label_handler,
        paint_router, styling_handler, treeview_handler,
    },
    error::{PlatformError, Result as PlatformResult},
    styling::StyleId,
    types::{AppEvent, ControlId, DockStyle, LayoutRule, MenuActionId, MessageSeverity, WindowId},
};

use windows::core::w;
use windows::{
    Win32::{
        Foundation::{
            COLORREF, ERROR_INVALID_WINDOW_HANDLE, GetLastError, HWND, LPARAM, LRESULT, POINT,
            RECT, WPARAM,
        },
        Graphics::Dwm::{DWMWINDOWATTRIBUTE, DwmSetWindowAttribute},
        Graphics::Gdi::{
            BeginPaint, CLIP_DEFAULT_PRECIS, COLOR_WINDOW, CreateFontIndirectW, CreateFontW,
            CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_GUI_FONT, DEFAULT_QUALITY, DT_CENTER,
            DT_HIDEPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint,
            FF_DONTCARE, FW_BOLD, FW_NORMAL, FillRect, GetDC, GetDeviceCaps, GetObjectW,
            GetStockObject, GetWindowDC, HBRUSH, HDC, HFONT, HGDIOBJ, InvalidateRect, LOGFONTW,
            LOGPIXELSY, MapWindowPoints, OUT_DEFAULT_PRECIS, OffsetRect, PAINTSTRUCT,
            RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow, ReleaseDC,
            SetBkColor, SetBkMode, SetTextColor, TRANSPARENT, UpdateWindow,
        },
        System::LibraryLoader::{GetProcAddress, LoadLibraryW},
        System::WindowsProgramming::MulDiv,
        UI::Controls::{
            DRAWITEMSTRUCT, NM_CLICK, NM_CUSTOMDRAW, NMHDR, ODS_HOTLIGHT, ODS_NOACCEL,
            ODS_SELECTED, SetWindowTheme, TVN_ITEMCHANGEDW,
        },
        UI::WindowsAndMessaging::*, // This list is massive, just import all of them.
    },
    core::{BOOL, HSTRING, PCSTR, PCWSTR},
};

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};

use log::warn;

// TOOD: Control IDs used by dialog_handler, kept here for visibility if dialog_handler needs them
// but ideally, they should be private to dialog_handler or within a shared constants scope for dialogs.
pub(crate) const ID_DIALOG_INPUT_EDIT: i32 = 3001;
pub(crate) const ID_DIALOG_INPUT_PROMPT_STATIC: i32 = 3002;
pub(crate) const ID_DIALOG_EXCLUDE_PATTERNS_EDIT: i32 = 3003;
pub(crate) const ID_DIALOG_EXCLUDE_PATTERNS_PROMPT_STATIC: i32 = 3004;

// Common control class names
pub(crate) const WC_STATIC: PCWSTR = windows::core::w!("STATIC");
// Common style constants
pub(crate) const SS_LEFT: WINDOW_STYLE = WINDOW_STYLE(0x00000000_u32);

// Custom application message for TreeView checkbox clicks.
// Defined here as it's part of the window message protocol that window_common handles.
pub(crate) const WM_APP_TREEVIEW_CHECKBOX_CLICKED: u32 = WM_APP + 0x100;
// Custom application message used to defer the MainWindowUISetupComplete event
// until after the Win32 message loop has started. This ensures controls like the
// TreeView have completed their creation and are ready for commands such as
// populating items with checkboxes.
pub(crate) const WM_APP_MAIN_WINDOW_UI_SETUP_COMPLETE: u32 = WM_APP + 0x101;
// Custom application messages for splitter drag events.
pub(crate) const WM_APP_SPLITTER_DRAGGING: u32 = WM_APP + 0x102;
pub(crate) const WM_APP_SPLITTER_DRAG_ENDED: u32 = WM_APP + 0x103;

// General UI constants
/// Default debounce delay for edit controls in milliseconds.
pub const INPUT_DEBOUNCE_MS: u32 = 300;

// Represents an invalid HWND, useful for initialization or checks.
pub(crate) const HWND_INVALID: HWND = HWND(std::ptr::null_mut());

const SUCCESS_CODE: LRESULT = LRESULT(0);
const UXTHEME_ORD_REFRESH_IMMERSIVE_COLOR_POLICY_STATE: usize = 104;
const UXTHEME_ORD_ALLOW_DARK_MODE_FOR_WINDOW: usize = 133;
const UXTHEME_ORD_SET_PREFERRED_APP_MODE: usize = 135;
const UXTHEME_ORD_FLUSH_MENU_THEMES: usize = 136;

/// Identifies the kind of a control so styles can be dispatched without Win32 class queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlKind {
    Button,
    ProgressBar,
    TreeView,
    Static,
    Edit,
    RichEdit,
    Splitter,
    ComboBox,
    RadioButton,
    CheckBox,
}

/*
 * Tracks control/window pairs whose next scroll notifications originate from our own commands.
 * The thread-local set prevents feedback loops when the platform mirrors app logic initiated scrolls.
 */
thread_local! {
    static PROGRAMMATIC_SCROLL_SUPPRESSIONS: RefCell<HashSet<(WindowId, ControlId)>> =
        RefCell::new(HashSet::new());
}

/*
 * RAII helper that marks a control's scroll updates as programmatic until dropped.
 * Keeps suppression lifetimes scoped so handlers outside the guard can forward genuine user events.
 */
#[derive(Debug)]
pub(crate) struct ProgrammaticScrollGuard {
    window_id: WindowId,
    control_id: ControlId,
}

impl ProgrammaticScrollGuard {
    pub(crate) fn new(window_id: WindowId, control_id: ControlId) -> Self {
        PROGRAMMATIC_SCROLL_SUPPRESSIONS.with(|set| {
            set.borrow_mut().insert((window_id, control_id));
        });
        Self {
            window_id,
            control_id,
        }
    }
}

impl Drop for ProgrammaticScrollGuard {
    fn drop(&mut self) {
        PROGRAMMATIC_SCROLL_SUPPRESSIONS.with(|set| {
            set.borrow_mut().remove(&(self.window_id, self.control_id));
        });
    }
}

fn is_scroll_event_suppressed(window_id: WindowId, control_id: ControlId) -> bool {
    PROGRAMMATIC_SCROLL_SUPPRESSIONS.with(|set| set.borrow().contains(&(window_id, control_id)))
}
/*
 * Holds native data associated with a specific window managed by the platform layer.
 * This includes the native window handle (`HWND`), a map of control IDs to their
 * `HWND`s, any control-specific states (like for the TreeView),
 * a map for menu item actions (`menu_action_map`),
 * a counter for generating unique menu item IDs (`next_menu_item_id_counter`),
 * a list of layout rules for positioning controls, and
 * severity information for new labels.
 */
#[derive(Debug)]
pub(crate) struct NativeWindowData {
    this_window_hwnd: HWND,
    logical_window_id: WindowId,
    // The specific internal state for the TreeView control if one exists.
    treeview_state: Option<treeview_handler::TreeViewInternalState>,
    // HWNDs for various controls (buttons, status bar, treeview, etc.)
    control_hwnd_map: HashMap<ControlId, HWND>,
    // Maps dynamically generated `i32` menu item IDs to their semantic `MenuActionId`.
    menu_action_map: HashMap<i32, MenuActionId>,
    // Maps a control's ID to the semantic StyleId applied to it.
    applied_styles: HashMap<ControlId, StyleId>,
    control_kinds: HashMap<ControlId, ControlKind>,
    // Counter to generate unique `i32` IDs for menu items that have an action.
    next_menu_item_id_counter: i32,
    // Layout rules for controls within this window.
    layout_rules: Option<Vec<LayoutRule>>,
    /// The current severity for each status label, keyed by its logical ID.
    label_severities: HashMap<ControlId, MessageSeverity>,
    status_bar_font: Option<HFONT>,
    treeview_new_item_font: Option<HFONT>,
    suppress_erasebkgnd: bool,
    last_layout_rects: RefCell<HashMap<ControlId, RECT>>,
    combo_dropdown_heal_attempted: HashSet<ControlId>,
}

impl NativeWindowData {
    pub(crate) fn new(logical_window_id: WindowId) -> Self {
        Self {
            this_window_hwnd: HWND_INVALID,
            logical_window_id,
            treeview_state: None,
            control_hwnd_map: HashMap::new(),
            menu_action_map: HashMap::new(),
            applied_styles: HashMap::new(),
            control_kinds: HashMap::new(),
            next_menu_item_id_counter: 30000,
            layout_rules: None,
            label_severities: HashMap::new(),
            status_bar_font: None,
            treeview_new_item_font: None,
            suppress_erasebkgnd: false,
            last_layout_rects: RefCell::new(HashMap::new()),
            combo_dropdown_heal_attempted: HashSet::new(),
        }
    }

    pub(crate) fn get_hwnd(&self) -> HWND {
        self.this_window_hwnd
    }

    pub(crate) fn set_hwnd(&mut self, hwnd: HWND) {
        self.this_window_hwnd = hwnd;
    }

    pub(crate) fn get_control_hwnd(&self, control_id: ControlId) -> Option<HWND> {
        self.control_hwnd_map.get(&control_id).copied()
    }

    pub(crate) fn register_control_hwnd(&mut self, control_id: ControlId, hwnd: HWND) {
        self.control_hwnd_map.insert(control_id, hwnd);
    }

    pub(crate) fn has_control(&self, control_id: ControlId) -> bool {
        self.control_hwnd_map.contains_key(&control_id)
    }

    pub(crate) fn has_treeview_state(&self) -> bool {
        self.treeview_state.is_some()
    }

    pub(crate) fn init_treeview_state(&mut self) {
        self.treeview_state = Some(treeview_handler::TreeViewInternalState::new());
    }

    pub(crate) fn take_treeview_state(
        &mut self,
    ) -> Option<treeview_handler::TreeViewInternalState> {
        self.treeview_state.take()
    }

    pub(crate) fn set_treeview_state(
        &mut self,
        state: Option<treeview_handler::TreeViewInternalState>,
    ) {
        self.treeview_state = state;
    }

    pub(crate) fn get_treeview_state(&self) -> Option<&treeview_handler::TreeViewInternalState> {
        self.treeview_state.as_ref()
    }

    pub(crate) fn apply_style_to_control(&mut self, control_id: ControlId, style_id: StyleId) {
        // [CDU-Styling-ApplyV1] Track which logical control has an applied `StyleId` so redraw hooks can resolve palette/font info.
        self.applied_styles.insert(control_id, style_id);
    }

    pub(crate) fn get_style_for_control(&self, control_id: ControlId) -> Option<StyleId> {
        self.applied_styles.get(&control_id).copied()
    }

    pub(crate) fn register_control_kind(&mut self, control_id: ControlId, kind: ControlKind) {
        self.control_kinds.insert(control_id, kind);
    }

    pub(crate) fn unregister_control_kind(&mut self, control_id: ControlId) {
        self.control_kinds.remove(&control_id);
    }

    pub(crate) fn get_control_kind(&self, control_id: ControlId) -> Option<ControlKind> {
        self.control_kinds.get(&control_id).copied()
    }

    fn effective_native_height_for_control(&self, control_id: ControlId, base_height: i32) -> i32 {
        if self.get_control_kind(control_id) == Some(ControlKind::ComboBox) {
            match self.get_control_hwnd(control_id) {
                Some(hwnd) => combobox_handler::compute_min_dropdown_height_px(hwnd, base_height),
                None => base_height.max(combobox_handler::fallback_min_dropdown_height_px()),
            }
        } else {
            base_height
        }
    }

    fn mark_dropdown_heal_attempted(&mut self, control_id: ControlId) -> bool {
        self.combo_dropdown_heal_attempted.insert(control_id)
    }

    fn clear_dropdown_heal_attempted(&mut self, control_id: ControlId) {
        self.combo_dropdown_heal_attempted.remove(&control_id);
    }

    pub(crate) fn find_control_id_by_hwnd(&self, hwnd: HWND) -> Option<ControlId> {
        self.control_hwnd_map
            .iter()
            .find_map(|(control_id, mapped_hwnd)| (*mapped_hwnd == hwnd).then_some(*control_id))
    }

    pub(crate) fn set_suppress_erasebkgnd(&mut self, suppress: bool) {
        self.suppress_erasebkgnd = suppress;
    }

    pub(crate) fn suppresses_erasebkgnd(&self) -> bool {
        self.suppress_erasebkgnd
    }

    fn generate_menu_item_id(&mut self) -> i32 {
        let id = self.next_menu_item_id_counter;
        self.next_menu_item_id_counter += 1;
        id
    }

    pub(crate) fn register_menu_action(&mut self, action_id: MenuActionId) -> i32 {
        let id = self.generate_menu_item_id();
        self.menu_action_map.insert(id, action_id);
        log::debug!(
            "CommandExecutor: Mapping menu action ID {:?} to command {} for window {:?}",
            action_id,
            id,
            self.logical_window_id
        );
        id
    }

    /*
     * Pure layout calculation for a group of child controls. Returns the
     * rectangle for each control without calling any Win32 APIs. The
     * algorithm mirrors the runtime layout engine and is recursively
     * applied by `apply_layout_rules_for_children`.
     * [CDU-LayoutSystemV1] Declarative docking rules are validated here without touching the Win32 APIs.
     */
    fn calculate_layout(parent_rect: RECT, rules: &[LayoutRule]) -> HashMap<ControlId, RECT> {
        let mut sorted = rules.to_vec();
        sorted.sort_by_key(|r| r.order);

        let mut result = HashMap::new();
        let mut current_available_rect = parent_rect;
        let mut fill_candidate: Option<&LayoutRule> = None;
        let mut proportional_fill_candidates: Vec<&LayoutRule> = Vec::new();

        for rule in &sorted {
            match rule.dock_style {
                DockStyle::Top | DockStyle::Bottom | DockStyle::Left | DockStyle::Right => {
                    let mut item_rect = RECT {
                        left: current_available_rect.left + rule.margin.3,
                        top: current_available_rect.top + rule.margin.0,
                        right: current_available_rect.right - rule.margin.1,
                        bottom: current_available_rect.bottom - rule.margin.2,
                    };
                    let size = rule.fixed_size.unwrap_or(0);
                    match rule.dock_style {
                        DockStyle::Top => {
                            item_rect.bottom = item_rect.top + size;
                            current_available_rect.top = item_rect.bottom + rule.margin.2;
                        }
                        DockStyle::Bottom => {
                            item_rect.top = item_rect.bottom - size;
                            current_available_rect.bottom = item_rect.top - rule.margin.0;
                        }
                        DockStyle::Left => {
                            item_rect.right = item_rect.left + size;
                            current_available_rect.left = item_rect.right + rule.margin.1;
                        }
                        DockStyle::Right => {
                            item_rect.left = item_rect.right - size;
                            current_available_rect.right = item_rect.left - rule.margin.3;
                        }
                        _ => unreachable!(),
                    }
                    result.insert(rule.control_id, item_rect);
                }
                DockStyle::Fill => {
                    if fill_candidate.is_none() {
                        fill_candidate = Some(rule);
                    }
                }
                DockStyle::ProportionalFill { .. } => {
                    proportional_fill_candidates.push(rule);
                }
                DockStyle::None => {}
            }
        }

        if !proportional_fill_candidates.is_empty() {
            let total_width_for_proportional =
                (current_available_rect.right - current_available_rect.left).max(0);
            let total_height_for_proportional =
                (current_available_rect.bottom - current_available_rect.top).max(0);
            let total_weight: f32 = proportional_fill_candidates
                .iter()
                .map(|r| match r.dock_style {
                    DockStyle::ProportionalFill { weight } => weight,
                    _ => 0.0,
                })
                .sum();
            if total_weight > 0.0 {
                let mut current_x = current_available_rect.left;
                for rule in proportional_fill_candidates {
                    if let DockStyle::ProportionalFill { weight } = rule.dock_style {
                        let proportion = weight / total_weight;
                        let item_width_allocation =
                            (total_width_for_proportional as f32 * proportion) as i32;
                        let final_x = current_x + rule.margin.3;
                        let final_y = current_available_rect.top + rule.margin.0;
                        let final_width =
                            (item_width_allocation - rule.margin.3 - rule.margin.1).max(0);
                        let final_height =
                            (total_height_for_proportional - rule.margin.0 - rule.margin.2).max(0);
                        result.insert(
                            rule.control_id,
                            RECT {
                                left: final_x,
                                top: final_y,
                                right: final_x + final_width,
                                bottom: final_y + final_height,
                            },
                        );
                        current_x += item_width_allocation;
                    }
                }
            }
        }

        if let Some(rule) = fill_candidate {
            let fill_rect = RECT {
                left: current_available_rect.left + rule.margin.3,
                top: current_available_rect.top + rule.margin.0,
                right: current_available_rect.right - rule.margin.1,
                bottom: current_available_rect.bottom - rule.margin.2,
            };
            result.insert(rule.control_id, fill_rect);
        }

        result
    }

    /*
     * Applies layout rules recursively for a parent and its children.
     * The heavy lifting is done by `calculate_layout`, which returns the
     * desired rectangles for each child. This function merely calls the
     * Win32 API to move the windows and recurses for nested containers.
     */
    fn apply_layout_rules_for_children(
        &self,
        parent_id_for_layout: Option<ControlId>,
        parent_rect: RECT,
    ) {
        log::trace!(
            "Applying layout for parent_id {parent_id_for_layout:?}, rect: {parent_rect:?}"
        );

        let all_window_rules = match &self.layout_rules {
            Some(rules) => rules,
            None => return, // No rules to apply
        };

        let mut child_rules: Vec<LayoutRule> = all_window_rules
            .iter()
            .filter(|r| r.parent_control_id == parent_id_for_layout)
            .cloned()
            .collect();
        if child_rules.is_empty() {
            return;
        }
        child_rules.sort_by_key(|r| r.order);

        if child_rules
            .iter()
            .filter(|r| r.dock_style == DockStyle::Fill)
            .count()
            > 1
        {
            log::warn!(
                "Layout: Multiple Fill controls for parent_id {parent_id_for_layout:?}. Using first."
            );
        }

        log::debug!(
            "[Layout] Applying layout: parent_id={parent_id_for_layout:?}, rules={}, rect={parent_rect:?}",
            child_rules.len()
        );

        let layout_map = NativeWindowData::calculate_layout(parent_rect, &child_rules);
        let parent_hwnd = if let Some(parent_id) = parent_id_for_layout {
            self.control_hwnd_map.get(&parent_id).copied()
        } else {
            Some(self.this_window_hwnd)
        };

        // Clear old rectangles for moved controls on the parent to avoid stale artifacts.
        if let Some(hwnd_parent) = parent_hwnd {
            let last_rects = self.last_layout_rects.borrow();
            for rule in &child_rules {
                let new_rect = match layout_map.get(&rule.control_id) {
                    Some(r) => r,
                    None => continue,
                };
                if let Some(old_rect) = last_rects.get(&rule.control_id)
                    && old_rect != new_rect
                {
                    unsafe {
                        _ = InvalidateRect(Some(hwnd_parent), Some(old_rect), true);
                    }
                }
            }
        }

        // Use deferred window positioning for flicker-free atomic updates
        unsafe {
            let hdwp_result = BeginDeferWindowPos(child_rules.len() as i32);
            let mut current_hdwp = match hdwp_result {
                Ok(hdwp) if !hdwp.is_invalid() => hdwp,
                _ => {
                    log::warn!("Layout: BeginDeferWindowPos failed, falling back to MoveWindow");
                    log::warn!(
                        "[Layout] BeginDeferWindowPos failed; fallback to MoveWindow (rules={})",
                        child_rules.len()
                    );
                    // Fallback to individual MoveWindow calls
                    for rule in &child_rules {
                        let rect = match layout_map.get(&rule.control_id) {
                            Some(r) => r,
                            None => continue,
                        };
                        let control_hwnd_opt = self.control_hwnd_map.get(&rule.control_id).copied();
                        if control_hwnd_opt.is_none() || control_hwnd_opt == Some(HWND_INVALID) {
                            continue;
                        }
                        let hwnd = control_hwnd_opt.unwrap();
                        let width = (rect.right - rect.left).max(0);
                        let height = (rect.bottom - rect.top).max(0);
                        let native_height =
                            self.effective_native_height_for_control(rule.control_id, height);
                        _ = MoveWindow(hwnd, rect.left, rect.top, width, native_height, true);
                    }
                    return;
                }
            };

            let mut moved_count = 0usize;
            for rule in &child_rules {
                let rect = match layout_map.get(&rule.control_id) {
                    Some(r) => r,
                    None => continue,
                };
                let control_hwnd_opt = self.control_hwnd_map.get(&rule.control_id).copied();
                if control_hwnd_opt.is_none() || control_hwnd_opt == Some(HWND_INVALID) {
                    log::warn!(
                        "Layout: HWND for control ID {} not found or invalid.",
                        rule.control_id.raw()
                    );
                    continue;
                }
                let hwnd = control_hwnd_opt.unwrap();
                let width = (rect.right - rect.left).max(0);
                let height = (rect.bottom - rect.top).max(0);
                let native_height =
                    self.effective_native_height_for_control(rule.control_id, height);

                // Use DeferWindowPos instead of MoveWindow for atomic repositioning
                // SWP_NOREDRAW suppresses individual redraws for flicker-free updates
                match DeferWindowPos(
                    current_hdwp,
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    width,
                    native_height,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
                ) {
                    Ok(new_hdwp) if !new_hdwp.is_invalid() => {
                        current_hdwp = new_hdwp;
                        moved_count += 1;
                    }
                    _ => {
                        log::warn!(
                            "Layout: DeferWindowPos failed for control ID {}",
                            rule.control_id.raw()
                        );
                    }
                }
            }

            // Apply all deferred moves atomically
            _ = EndDeferWindowPos(current_hdwp);

            log::debug!(
                "[Layout] Deferred layout applied: parent_id={parent_id_for_layout:?}, moved={moved_count}"
            );

            // Trigger a single redraw of the parent window to show all changes at once
            // This eliminates flicker from individual control repaints
            // Invalidate container panels (Static with children) without erase to refresh background.
            for rule in &child_rules {
                let has_children = all_window_rules
                    .iter()
                    .any(|r_child| r_child.parent_control_id == Some(rule.control_id));
                if !has_children {
                    continue;
                }
                let kind = self.control_kinds.get(&rule.control_id).copied();
                if !matches!(kind, Some(ControlKind::Static)) {
                    continue;
                }
                let control_hwnd_opt = self.control_hwnd_map.get(&rule.control_id).copied();
                if control_hwnd_opt.is_none() || control_hwnd_opt == Some(HWND_INVALID) {
                    continue;
                }
                let hwnd = control_hwnd_opt.unwrap();
                _ = InvalidateRect(Some(hwnd), None, true);
                _ = UpdateWindow(hwnd);
                log::debug!(
                    "[Layout] Invalidated panel control id={} kind={kind:?} erase=true hwnd={hwnd:?}",
                    rule.control_id.raw()
                );
            }

            // Invalidate leaf content controls (TreeView/Edit/Splitter/Static) to refresh content
            // without triggering a full parent erase.
            let mut invalidated = 0usize;
            for rule in &child_rules {
                let has_children = all_window_rules
                    .iter()
                    .any(|r_child| r_child.parent_control_id == Some(rule.control_id));
                if has_children {
                    continue;
                }
                let kind = self.control_kinds.get(&rule.control_id).copied();
                if !matches!(
                    kind,
                    Some(
                        ControlKind::TreeView
                            | ControlKind::Edit
                            | ControlKind::RichEdit
                            | ControlKind::Splitter
                            | ControlKind::Static
                            | ControlKind::ComboBox
                            | ControlKind::ProgressBar
                    )
                ) {
                    continue;
                }
                let control_hwnd_opt = self.control_hwnd_map.get(&rule.control_id).copied();
                if control_hwnd_opt.is_none() || control_hwnd_opt == Some(HWND_INVALID) {
                    continue;
                }
                let hwnd = control_hwnd_opt.unwrap();
                let is_header_label = matches!(
                    (kind, self.get_style_for_control(rule.control_id)),
                    (Some(ControlKind::Static), Some(StyleId::HeaderLabel))
                );
                let suppress_header_erase = self.suppresses_erasebkgnd() && is_header_label;
                let erase = if suppress_header_erase {
                    false
                } else {
                    matches!(
                        kind,
                        Some(ControlKind::TreeView | ControlKind::Edit | ControlKind::Static)
                            | Some(ControlKind::RichEdit)
                    )
                };
                if matches!(kind, Some(ControlKind::TreeView)) {
                    _ = SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED,
                    );
                }
                _ = InvalidateRect(Some(hwnd), None, erase);
                _ = UpdateWindow(hwnd);
                log::debug!(
                    "[Layout] Invalidated leaf control id={} kind={kind:?} erase={erase} hwnd={hwnd:?}",
                    rule.control_id.raw()
                );
                invalidated += 1;
            }
            log::debug!(
                "[Layout] Leaf invalidation completed (parent_id={parent_id_for_layout:?}, count={invalidated})"
            );
        }

        // Persist latest rects for future invalidation.
        {
            let mut last_rects = self.last_layout_rects.borrow_mut();
            for (control_id, rect) in &layout_map {
                last_rects.insert(*control_id, *rect);
            }
        }

        // Recursively apply layout to children after all moves are complete
        for rule in &child_rules {
            if all_window_rules
                .iter()
                .any(|r_child| r_child.parent_control_id == Some(rule.control_id))
            {
                let rect = match layout_map.get(&rule.control_id) {
                    Some(r) => r,
                    None => continue,
                };
                let width = (rect.right - rect.left).max(0);
                let height = (rect.bottom - rect.top).max(0);
                let panel_client_rect = RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                };
                self.apply_layout_rules_for_children(Some(rule.control_id), panel_client_rect);
            }
        }
    }

    pub(crate) fn get_menu_action(&self, menu_id: i32) -> Option<MenuActionId> {
        self.menu_action_map.get(&menu_id).copied()
    }

    #[cfg(test)]
    pub(crate) fn iter_menu_actions(&self) -> impl Iterator<Item = (&i32, &MenuActionId)> {
        self.menu_action_map.iter()
    }

    #[cfg(test)]
    pub(crate) fn menu_action_count(&self) -> usize {
        self.menu_action_map.len()
    }

    #[cfg(test)]
    pub(crate) fn get_next_menu_item_id_counter(&self) -> i32 {
        self.next_menu_item_id_counter
    }

    pub(crate) fn define_layout(&mut self, rules: Vec<LayoutRule>) -> PlatformResult<()> {
        Self::validate_layout_rules(&rules)?;
        self.layout_rules = Some(rules);
        Ok(())
    }

    fn validate_layout_rules(rules: &[LayoutRule]) -> PlatformResult<()> {
        let mut fill_by_parent: HashMap<Option<ControlId>, Vec<ControlId>> = HashMap::new();
        for rule in rules {
            if rule.dock_style == DockStyle::Fill {
                fill_by_parent
                    .entry(rule.parent_control_id)
                    .or_default()
                    .push(rule.control_id);
            }
        }

        for (parent_id, fill_controls) in fill_by_parent {
            if fill_controls.len() > 1 {
                let parent_desc = parent_id
                    .map(|id| format!("control {}", id.raw()))
                    .unwrap_or_else(|| "main window".to_string());
                let control_ids = fill_controls
                    .iter()
                    .map(|id| id.raw().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(PlatformError::OperationFailed(format!(
                    "DefineLayout rejected: parent {parent_desc} has multiple DockStyle::Fill children ({control_ids}). CommanDuctUI supports exactly one Fill child per parent."
                )));
            }
        }

        Ok(())
    }

    /*
     * Recalculates the window's layout using the stored rules and immediately applies
     * the resulting rectangles to every registered control. Centralizing this logic
     * inside `NativeWindowData` keeps the layout internals hidden from callers.
     *
     * The method exits early when prerequisites are missing, and logs an error if the
     * client rectangle cannot be retrieved. It remains safe to call repeatedly because
     * it is effectively a no-op when the layout data or window handle is invalid.
     */
    pub(crate) fn recalculate_and_apply_layout(&self) {
        if self.layout_rules.is_none() {
            return;
        }

        if self.this_window_hwnd.is_invalid() {
            log::warn!(
                "Layout: HWND invalid for recalculation on WinID {:?}.",
                self.logical_window_id
            );
            return;
        }

        let mut client_rect = RECT::default();
        if unsafe { GetClientRect(self.this_window_hwnd, &mut client_rect) }.is_err() {
            log::error!(
                "Layout: GetClientRect failed for WinID {:?}: {:?}",
                self.logical_window_id,
                unsafe { GetLastError() }
            );
            return;
        }

        log::trace!(
            "Layout: Applying layout with client_rect {client_rect:?} for WinID {:?}.",
            self.logical_window_id
        );
        self.apply_layout_rules_for_children(None, client_rect);

        // Force a full post-layout redraw pass. Dynamic Prompt Lab mode/section toggles can
        // otherwise leave stale pixels from previously larger control regions.
        unsafe {
            _ = RedrawWindow(
                Some(self.this_window_hwnd),
                None,
                None,
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW,
            );
        }
    }

    pub(crate) fn set_label_severity(&mut self, label_id: ControlId, severity: MessageSeverity) {
        self.label_severities.insert(label_id, severity);
    }

    pub(crate) fn get_label_severity(&self, label_id: ControlId) -> Option<MessageSeverity> {
        self.label_severities.get(&label_id).copied()
    }

    pub(crate) fn ensure_status_bar_font(&mut self) {
        if self.status_bar_font.is_some() {
            return;
        }

        let font_name_hstring = HSTRING::from("Segoe UI");
        let font_point_size = 9;
        let hdc_screen = unsafe { GetDC(None) };
        let logical_font_height = if !hdc_screen.is_invalid() {
            let height = -unsafe {
                MulDiv(
                    font_point_size,
                    GetDeviceCaps(Some(hdc_screen), LOGPIXELSY),
                    72,
                )
            };
            unsafe { ReleaseDC(None, hdc_screen) };
            height
        } else {
            -font_point_size
        };

        let h_font = unsafe {
            CreateFontW(
                logical_font_height,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                DEFAULT_QUALITY,
                FF_DONTCARE.0 as u32,
                &font_name_hstring,
            )
        };

        if h_font.is_invalid() {
            log::error!("Platform: Failed to create status bar font: {:?}", unsafe {
                GetLastError()
            });
            self.status_bar_font = None;
        } else {
            log::debug!(
                "Platform: Status bar font created: {:?} for window {:?}",
                h_font,
                self.logical_window_id
            );
            self.status_bar_font = Some(h_font);
        }
    }

    pub(crate) fn get_status_bar_font(&self) -> Option<HFONT> {
        self.status_bar_font
    }

    fn cleanup_status_bar_font(&mut self) {
        if let Some(h_font) = self.status_bar_font.take()
            && !h_font.is_invalid()
        {
            log::debug!(
                "Deleting status bar font {:?} for WinID {:?}",
                h_font,
                self.logical_window_id
            );
            unsafe {
                let _ = DeleteObject(HGDIOBJ(h_font.0));
            }
        }
    }

    /*
     * Ensures the TreeView "new item" font exists. The font mirrors the default GUI font but
     * forces a bold, italic variant so the indicator styling is consistent with system metrics.
     * This method is idempotent and cheap to call; once the font exists it simply returns.
     */
    pub(crate) fn ensure_treeview_new_item_font(&mut self) {
        if self.treeview_new_item_font.is_some() {
            return;
        }

        let stock_font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
        if stock_font.0.is_null() {
            log::error!(
                "Platform: DEFAULT_GUI_FONT unavailable while creating TreeView indicator font for {:?}.",
                self.logical_window_id
            );
            return;
        }

        let mut base_log_font = LOGFONTW::default();
        let copy_result = unsafe {
            GetObjectW(
                stock_font,
                std::mem::size_of::<LOGFONTW>() as i32,
                Some(&mut base_log_font as *mut _ as *mut c_void),
            )
        };

        if copy_result == 0 {
            log::error!(
                "Platform: GetObjectW failed while cloning default GUI font for TreeView indicator (WinID {:?}). LastError={:?}",
                self.logical_window_id,
                unsafe { GetLastError() }
            );
            return;
        }

        base_log_font.lfWeight = FW_BOLD.0 as i32;
        base_log_font.lfItalic = 1;

        let new_font = unsafe { CreateFontIndirectW(&base_log_font) };
        if new_font.is_invalid() {
            log::error!(
                "Platform: CreateFontIndirectW failed for TreeView indicator font (WinID {:?}). LastError={:?}",
                self.logical_window_id,
                unsafe { GetLastError() }
            );
            return;
        }

        log::debug!(
            "Platform: TreeView 'new item' font created {:?} for WinID {:?}.",
            new_font,
            self.logical_window_id
        );
        self.treeview_new_item_font = Some(new_font);
    }

    pub(crate) fn get_treeview_new_item_font(&self) -> Option<HFONT> {
        self.treeview_new_item_font
    }

    fn cleanup_treeview_new_item_font(&mut self) {
        if let Some(h_font) = self.treeview_new_item_font.take()
            && !h_font.is_invalid()
        {
            log::debug!(
                "Deleting TreeView 'new item' font {:?} for WinID {:?}.",
                h_font,
                self.logical_window_id
            );
            unsafe {
                let _ = DeleteObject(HGDIOBJ(h_font.0));
            }
        }
    }
}

impl Drop for NativeWindowData {
    fn drop(&mut self) {
        self.cleanup_status_bar_font();
        self.cleanup_treeview_new_item_font();
        log::debug!(
            "NativeWindowData for WinID {:?} dropped, resources cleaned up.",
            self.logical_window_id
        );
    }
}

// Context passed during window creation to associate Win32ApiInternalState with HWND.
struct WindowCreationContext {
    internal_state_arc: Arc<Win32ApiInternalState>,
    window_id: WindowId,
}

/*
 * Registers the main window class for the application if not already registered.
 * This function defines the common properties for all windows created by this
 * platform layer, including the window procedure (`facade_wnd_proc_router`).
 */
pub(crate) fn register_window_class(
    internal_state: &Arc<Win32ApiInternalState>,
) -> PlatformResult<()> {
    let class_name_hstring = HSTRING::from(format!(
        "{}_PlatformWindowClass",
        internal_state.app_name_for_class()
    ));
    let class_name_pcwstr = PCWSTR(class_name_hstring.as_ptr());

    unsafe {
        let mut wc_test = WNDCLASSEXW::default();
        if GetClassInfoExW(
            Some(internal_state.h_instance()),
            class_name_pcwstr,
            &mut wc_test,
        )
        .is_ok()
        {
            log::debug!(
                "Platform: Window class '{}' already registered.",
                internal_state.app_name_for_class()
            );
            return Ok(());
        }

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
            lpfnWndProc: Some(facade_wnd_proc_router),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: internal_state.h_instance(),
            hIcon: LoadIconW(None, IDI_APPLICATION)?,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut c_void),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name_pcwstr,
            hIconSm: LoadIconW(None, IDI_APPLICATION)?,
        };

        if RegisterClassExW(&wc) == 0 {
            let error = GetLastError();
            log::error!("Platform: RegisterClassExW failed: {error:?}");
            Err(PlatformError::InitializationFailed(format!(
                "RegisterClassExW failed: {error:?}"
            )))
        } else {
            log::debug!(
                "Platform: Window class '{}' registered successfully.",
                internal_state.app_name_for_class()
            );
            Ok(())
        }
    }
}

/*
 * Creates a native Win32 window.
 * Uses `CreateWindowExW` and passes `WindowCreationContext` via `lpCreateParams`.
 */
pub(crate) fn create_native_window(
    internal_state_arc: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    title: &str,
    width: i32,
    height: i32,
) -> PlatformResult<HWND> {
    let class_name_hstring = HSTRING::from(format!(
        "{}_PlatformWindowClass",
        internal_state_arc.app_name_for_class()
    ));

    let creation_context = Box::new(WindowCreationContext {
        internal_state_arc: Arc::clone(internal_state_arc),
        window_id,
    });

    unsafe {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            &class_name_hstring,                   // Window class name
            &HSTRING::from(title),                 // Window title
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN, // Common window style + clip children
            CW_USEDEFAULT,                         // Default X position
            CW_USEDEFAULT,                         // Default Y position
            width,                                 // Width
            height,                                // Height
            None,                                  // Parent window (None for top-level)
            None,                                  // Menu (None for no default menu)
            Some(internal_state_arc.h_instance()), // Application instance
            Some(Box::into_raw(creation_context) as *mut c_void), // lParam for WM_CREATE/WM_NCCREATE
        )?; // Returns Result<HWND, Error>, so ? operator handles error conversion

        Ok(hwnd)
    }
}

/*
 * Main window procedure router. Retrieves `WindowCreationContext` and calls
 * `handle_window_message` on `Win32ApiInternalState`.
 */
unsafe extern "system" fn facade_wnd_proc_router(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let context_ptr = if msg == WM_NCCREATE {
        let create_struct = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        let context_raw_ptr = create_struct.lpCreateParams as *mut WindowCreationContext;
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, context_raw_ptr as isize) };
        context_raw_ptr
    } else {
        unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowCreationContext }
    };

    if context_ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }

    let context = unsafe { &*context_ptr };
    let internal_state_arc = &context.internal_state_arc;
    let window_id = context.window_id;

    let result = internal_state_arc.handle_window_message(hwnd, msg, wparam, lparam, window_id);

    if msg == WM_NCDESTROY {
        let _ = unsafe { Box::from_raw(context_ptr) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
    }
    result
}

#[inline]
pub(crate) fn loword_from_wparam(wparam: WPARAM) -> i32 {
    (wparam.0 & 0xFFFF) as i32
}
#[inline]
pub(crate) fn highord_from_wparam(wparam: WPARAM) -> i32 {
    (wparam.0 >> 16) as i32
}
#[inline]
pub(crate) fn loword_from_lparam(lparam: LPARAM) -> i32 {
    (lparam.0 & 0xFFFF) as i32
}
#[inline]
pub(crate) fn hiword_from_lparam(lparam: LPARAM) -> i32 {
    ((lparam.0 >> 16) & 0xFFFF) as i32
}

/// App-level dark mode initialization.
///
/// Must be called **before** any window is created (`CreateWindowExW`).
/// Sets `SetPreferredAppMode(AllowDark)`, refreshes the immersive color
/// policy, and flushes menu themes so Windows knows the process opts into
/// dark rendering for title bars, menus, and scrollbars.
pub(crate) fn init_app_dark_mode() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| unsafe {
        let module = match LoadLibraryW(w!("uxtheme.dll")) {
            Ok(m) => m,
            Err(err) => {
                log::debug!("Failed to load uxtheme.dll for app-level dark mode: {err:?}");
                return;
            }
        };

        // SetPreferredAppMode(AllowDark) — ordinal 135
        if let Some(ptr) = get_uxtheme_proc_address(module, UXTHEME_ORD_SET_PREFERRED_APP_MODE) {
            let set_preferred: SetPreferredAppModeFn =
                std::mem::transmute::<*const c_void, SetPreferredAppModeFn>(ptr);
            let _ = set_preferred(PreferredAppMode::AllowDark);
            log::debug!("Dark mode: SetPreferredAppMode(AllowDark) succeeded.");
        }

        // RefreshImmersiveColorPolicyState — ordinal 104
        if let Some(ptr) =
            get_uxtheme_proc_address(module, UXTHEME_ORD_REFRESH_IMMERSIVE_COLOR_POLICY_STATE)
        {
            let refresh: RefreshImmersiveColorPolicyStateFn =
                std::mem::transmute::<*const c_void, RefreshImmersiveColorPolicyStateFn>(ptr);
            refresh();
            log::debug!("Dark mode: RefreshImmersiveColorPolicyState succeeded.");
        }

        // FlushMenuThemes — ordinal 136
        if let Some(ptr) = get_uxtheme_proc_address(module, UXTHEME_ORD_FLUSH_MENU_THEMES) {
            let flush: FlushMenuThemesFn =
                std::mem::transmute::<*const c_void, FlushMenuThemesFn>(ptr);
            flush();
            log::debug!("Dark mode: FlushMenuThemes succeeded.");
        }
    });
}

/// Enables dark mode and forces classic rendering (empty theme) on button-like controls.
///
/// This is the canonical setup path for RadioButton and CheckBox controls: it calls
/// `try_enable_dark_mode` then `SetWindowTheme("", "")` so that `WM_CTLCOLORBTN` /
/// `WM_CTLCOLORSTATIC` messages are delivered to the parent and our palette is applied.
/// Using a single helper here prevents the split between creation-time dark-mode enablement
/// and style-application-time classic rendering that caused previous dark-theme regressions.
pub(crate) fn apply_button_dark_mode_classic_render(hwnd: HWND) {
    try_enable_dark_mode(hwnd);
    unsafe {
        let empty = windows::core::HSTRING::new();
        let _ = SetWindowTheme(hwnd, &empty, &empty);
    }
}

/// Best-effort enablement of dark mode for non-client areas (notably scrollbars) on supported OS builds.
pub(crate) fn try_enable_dark_mode(hwnd: HWND) {
    try_enable_dark_menu_theme_support(hwnd);
    unsafe {
        let enable_dark: i32 = 1;
        const DWMWA_USE_IMMERSIVE_DARK_MODE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(20);
        // Try primary attribute ID
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &enable_dark as *const _ as *const _,
            std::mem::size_of_val(&enable_dark) as u32,
        );
        // Some builds expect 19; attempt as secondary.
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(19),
            &enable_dark as *const _ as *const _,
            std::mem::size_of_val(&enable_dark) as u32,
        );
        // Explorer dark theme often yields dark scrollbars on common controls.
        let _ = SetWindowTheme(hwnd, w!("DarkMode_Explorer"), None);
    }
}

#[repr(i32)]
#[derive(Clone, Copy)]
enum PreferredAppMode {
    AllowDark = 1,
}

type SetPreferredAppModeFn = unsafe extern "system" fn(PreferredAppMode) -> PreferredAppMode;
type FlushMenuThemesFn = unsafe extern "system" fn();
type RefreshImmersiveColorPolicyStateFn = unsafe extern "system" fn();
type AllowDarkModeForWindowFn = unsafe extern "system" fn(HWND, BOOL) -> BOOL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DarkModeUxThemeOrdinals {
    allow_dark_mode_for_window: bool,
    set_preferred_app_mode: bool,
    flush_menu_themes: bool,
}

impl DarkModeUxThemeOrdinals {
    fn has_any(self) -> bool {
        self.allow_dark_mode_for_window || self.set_preferred_app_mode || self.flush_menu_themes
    }
}

fn resolve_dark_mode_uxtheme_ordinals(
    has_ordinal: impl Fn(usize) -> bool,
) -> DarkModeUxThemeOrdinals {
    DarkModeUxThemeOrdinals {
        allow_dark_mode_for_window: has_ordinal(UXTHEME_ORD_ALLOW_DARK_MODE_FOR_WINDOW),
        set_preferred_app_mode: has_ordinal(UXTHEME_ORD_SET_PREFERRED_APP_MODE),
        flush_menu_themes: has_ordinal(UXTHEME_ORD_FLUSH_MENU_THEMES),
    }
}

fn get_uxtheme_proc_address(
    module: windows::Win32::Foundation::HMODULE,
    ordinal: usize,
) -> Option<*const c_void> {
    unsafe { GetProcAddress(module, PCSTR(ordinal as *const u8)) }.map(|func| func as *const c_void)
}

fn try_enable_dark_menu_theme_support(hwnd: HWND) {
    static ALLOW_DARK_MODE_FOR_WINDOW_PTR: OnceLock<Option<AllowDarkModeForWindowFn>> =
        OnceLock::new();

    let maybe_allow_window_dark = ALLOW_DARK_MODE_FOR_WINDOW_PTR.get_or_init(|| unsafe {
        let module = match LoadLibraryW(w!("uxtheme.dll")) {
            Ok(module) => module,
            Err(err) => {
                log::debug!("Failed to load uxtheme.dll for dark menu support: {err:?}");
                return None;
            }
        };

        let ordinals = resolve_dark_mode_uxtheme_ordinals(|ordinal| {
            get_uxtheme_proc_address(module, ordinal).is_some()
        });

        if !ordinals.has_any() {
            log::debug!("uxtheme dark menu ordinals are unavailable on this OS build.");
            return None;
        }

        if let Some(set_preferred_ptr) =
            get_uxtheme_proc_address(module, UXTHEME_ORD_SET_PREFERRED_APP_MODE)
        {
            let set_preferred: SetPreferredAppModeFn =
                std::mem::transmute::<*const c_void, SetPreferredAppModeFn>(set_preferred_ptr);
            let _ = set_preferred(PreferredAppMode::AllowDark);
        }

        if let Some(flush_menu_themes_ptr) =
            get_uxtheme_proc_address(module, UXTHEME_ORD_FLUSH_MENU_THEMES)
        {
            let flush_menu_themes: FlushMenuThemesFn =
                std::mem::transmute::<*const c_void, FlushMenuThemesFn>(flush_menu_themes_ptr);
            flush_menu_themes();
        }

        get_uxtheme_proc_address(module, UXTHEME_ORD_ALLOW_DARK_MODE_FOR_WINDOW).map(
            |allow_dark_mode_for_window_ptr| {
                std::mem::transmute::<*const c_void, AllowDarkModeForWindowFn>(
                    allow_dark_mode_for_window_ptr,
                )
            },
        )
    });

    if let Some(allow_dark_mode_for_window) = maybe_allow_window_dark {
        unsafe {
            let _ = allow_dark_mode_for_window(hwnd, true.into());
        }
        // Re-flush menu themes so Windows re-evaluates the menu bar
        // now that per-window dark mode has been enabled.
        flush_menu_themes_if_available();
    }
}

fn flush_menu_themes_if_available() {
    static FLUSH_FN: OnceLock<Option<FlushMenuThemesFn>> = OnceLock::new();
    let maybe_flush = FLUSH_FN.get_or_init(|| unsafe {
        let module = LoadLibraryW(w!("uxtheme.dll")).ok()?;
        get_uxtheme_proc_address(module, UXTHEME_ORD_FLUSH_MENU_THEMES)
            .map(|ptr| std::mem::transmute::<*const c_void, FlushMenuThemesFn>(ptr))
    });
    if let Some(flush) = maybe_flush {
        unsafe { flush() };
    }
}

// ---------------------------------------------------------------------------
// Undocumented UAH (User-Action-Handler) messages for painting the menu bar.
// Windows sends these to the owning window so it can custom-draw the menu bar
// strip.  Without handling them the bar stays light even when dark mode is on.
// ---------------------------------------------------------------------------
const WM_UAHDRAWMENU: u32 = 0x0091;
const WM_UAHDRAWMENUITEM: u32 = 0x0092;

/// Win32 `OBJID_MENU` (avoids pulling in `Win32_UI_Accessibility`).
const OBJID_MENU_BAR: i32 = -3;

/// Mirrors the undocumented `UAHMENU` structure Windows passes via `lParam`.
#[repr(C)]
struct UahMenu {
    hmenu: HMENU,
    hdc: HDC,
    _dw_flags: u32,
}

/// Mirrors the undocumented `UAHMENUITEM` that follows the `UAHMENU` inside
/// the `UAHDRAWMENUITEM` blob.
#[repr(C)]
struct UahMenuItem {
    i_position: i32,
    _dw_flags: u32,
}

/// Full `lParam` payload for `WM_UAHDRAWMENUITEM`.
#[repr(C)]
struct UahDrawMenuItem {
    dis: DRAWITEMSTRUCT,
    um: UahMenu,
    umi: UahMenuItem,
}

/// Dark-mode colours used for the menu bar painting.
struct MenuBarColors {
    bar_bg: COLORREF,
    text_normal: COLORREF,
    hot_bg: COLORREF,
    pushed_bg: COLORREF,
}

impl MenuBarColors {
    /// Derive from the `MainWindowBackground` style, falling back to sensible
    /// dark defaults when no style is set.
    fn from_state(state: &Win32ApiInternalState) -> Self {
        use super::controls::styling_handler::color_to_colorref;
        use super::styling_primitives::Color;

        let default_bg = Color {
            r: 0x2E,
            g: 0x32,
            b: 0x39,
        };
        let default_text = Color {
            r: 0xE0,
            g: 0xE5,
            b: 0xEC,
        };

        if let Some(parsed) = state.get_parsed_style(StyleId::MainWindowBackground) {
            let bg_color = parsed.background_color.as_ref().unwrap_or(&default_bg);
            let txt_color = parsed.text_color.as_ref().unwrap_or(&default_text);
            let bg = color_to_colorref(bg_color);
            MenuBarColors {
                bar_bg: bg,
                text_normal: color_to_colorref(txt_color),
                hot_bg: COLORREF(lighten_colorref(bg, 20)),
                pushed_bg: COLORREF(lighten_colorref(bg, 35)),
            }
        } else {
            // Reasonable dark fallback.
            let bg = color_to_colorref(&default_bg);
            MenuBarColors {
                bar_bg: bg,
                text_normal: color_to_colorref(&default_text),
                hot_bg: COLORREF(lighten_colorref(bg, 20)),
                pushed_bg: COLORREF(lighten_colorref(bg, 35)),
            }
        }
    }
}

/// Brighten each channel of a `COLORREF` by `amount` (clamped to 255).
fn lighten_colorref(c: COLORREF, amount: u32) -> u32 {
    let r = (c.0 & 0xFF).min(255 - amount) + amount;
    let g = ((c.0 >> 8) & 0xFF).min(255 - amount) + amount;
    let b = ((c.0 >> 16) & 0xFF).min(255 - amount) + amount;
    r | (g << 8) | (b << 16)
}

/// Fill the entire menu bar background (`WM_UAHDRAWMENU`).
unsafe fn paint_dark_menu_bar(hwnd: HWND, hdc: HDC, bar_bg: COLORREF) {
    unsafe {
        let mut mbi = MENUBARINFO {
            cbSize: std::mem::size_of::<MENUBARINFO>() as u32,
            ..Default::default()
        };
        if GetMenuBarInfo(hwnd, OBJECT_IDENTIFIER(OBJID_MENU_BAR), 0, &mut mbi).is_err() {
            return;
        }
        let mut rc_window = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rc_window);
        let mut rc_bar = mbi.rcBar;
        let _ = OffsetRect(&mut rc_bar, -rc_window.left, -rc_window.top);
        rc_bar.top -= 1;
        let brush = CreateSolidBrush(bar_bg);
        FillRect(hdc, &rc_bar, brush);
        let _ = DeleteObject(brush.into());
    }
}

/// Draw a single menu bar item (`WM_UAHDRAWMENUITEM`).
unsafe fn paint_dark_menu_bar_item(udmi: &UahDrawMenuItem, colors: &MenuBarColors) {
    unsafe {
        // Fetch menu item text.
        let mut buf = [0u16; 256];
        let mut mii = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_STRING,
            dwTypeData: windows::core::PWSTR(buf.as_mut_ptr()),
            cch: (buf.len() - 1) as u32,
            ..std::mem::zeroed()
        };
        let _ = GetMenuItemInfoW(udmi.um.hmenu, udmi.umi.i_position as u32, true, &mut mii);

        // Determine background brush.
        let item_state = udmi.dis.itemState;
        let bg_color = if (item_state.0 & ODS_SELECTED.0) != 0 {
            colors.pushed_bg
        } else if (item_state.0 & ODS_HOTLIGHT.0) != 0 {
            colors.hot_bg
        } else {
            colors.bar_bg
        };

        let brush = CreateSolidBrush(bg_color);
        FillRect(udmi.um.hdc, &udmi.dis.rcItem, brush);
        let _ = DeleteObject(brush.into());

        // Draw the text.
        let text_color = colors.text_normal;
        SetBkMode(udmi.um.hdc, TRANSPARENT);
        SetTextColor(udmi.um.hdc, text_color);

        let mut dt_flags = DT_CENTER | DT_SINGLELINE | DT_VCENTER;
        if (item_state.0 & ODS_NOACCEL.0) != 0 {
            dt_flags |= DT_HIDEPREFIX;
        }
        let mut rc = udmi.dis.rcItem;
        DrawTextW(udmi.um.hdc, &mut buf[..mii.cch as usize], &mut rc, dt_flags);
    }
}

/// Paint over the 1-px bright line Windows leaves between menu bar and client.
unsafe fn draw_dark_menu_nc_bottom_line(hwnd: HWND, bar_bg: COLORREF) {
    unsafe {
        let mut mbi = MENUBARINFO {
            cbSize: std::mem::size_of::<MENUBARINFO>() as u32,
            ..Default::default()
        };
        if GetMenuBarInfo(hwnd, OBJECT_IDENTIFIER(OBJID_MENU_BAR), 0, &mut mbi).is_err() {
            return;
        }
        let mut rc_client = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc_client);
        let points = std::slice::from_raw_parts_mut(&mut rc_client as *mut RECT as *mut POINT, 2);
        MapWindowPoints(Some(hwnd), None, points);
        let mut rc_window = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rc_window);
        let _ = OffsetRect(&mut rc_client, -rc_window.left, -rc_window.top);
        let rc_line = RECT {
            left: rc_client.left,
            top: rc_client.top - 1,
            right: rc_client.right,
            bottom: rc_client.top,
        };
        let hdc = GetWindowDC(Some(hwnd));
        let brush = CreateSolidBrush(bar_bg);
        FillRect(hdc, &rc_line, brush);
        let _ = DeleteObject(brush.into());
        ReleaseDC(Some(hwnd), hdc);
    }
}

impl Win32ApiInternalState {
    /*
     * Handles window messages for a specific window instance.
     * This method is called by `facade_wnd_proc_router` and processes
     * relevant messages. It translates them into `AppEvent`s to be sent to the
     * application logic or performs direct actions by dispatching to control handlers.
     * It may also override the default message result (`lresult_override`).
     */
    fn handle_window_message(
        self: &Arc<Self>,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        window_id: WindowId,
    ) -> LRESULT {
        let mut event_to_send: Option<AppEvent> = None;
        let mut lresult_override: Option<LRESULT> = None;

        match msg {
            WM_CREATE => {
                self.handle_wm_create(hwnd, wparam, lparam, window_id);
            }
            WM_SIZE => {
                event_to_send = self.handle_wm_size(hwnd, wparam, lparam, window_id);
            }
            WM_COMMAND => {
                event_to_send = self.handle_wm_command(hwnd, wparam, lparam, window_id);
            }
            WM_DRAWITEM => {
                let draw_item_struct = lparam.0 as *const DRAWITEMSTRUCT;
                lresult_override =
                    button_handler::handle_wm_drawitem(self, window_id, draw_item_struct);
            }
            WM_TIMER => {
                event_to_send = self.handle_wm_timer(hwnd, wparam, lparam, window_id);
            }
            WM_ERASEBKGND => {
                lresult_override = Some(self.handle_wm_erasebkgnd(hwnd, wparam, lparam, window_id));
            }
            WM_CLOSE => {
                log::debug!(
                    "WM_CLOSE received for WinID {window_id:?}. Generating WindowCloseRequestedByUser."
                );
                // [CDU-WindowLifecycleEventsV1] WM_CLOSE translates into a declarative `WindowCloseRequestedByUser` event before the native destruction proceeds.
                event_to_send = Some(AppEvent::WindowCloseRequestedByUser { window_id });
                lresult_override = Some(SUCCESS_CODE);
            }
            WM_DESTROY => {
                // DO NOT remove window data here. Just notify the app logic.
                log::debug!("WM_DESTROY received for WinID {window_id:?}. Notifying app logic.");
                event_to_send = Some(AppEvent::WindowDestroyed { window_id });
            }
            WM_NCDESTROY => {
                // This is the FINAL message. Now it's safe to clean up.
                log::debug!(
                    "WM_NCDESTROY received for WinID {window_id:?}. Initiating final cleanup."
                );
                self.remove_window_data(window_id);
                self.check_if_should_quit_after_window_close();
            }
            WM_PAINT => {
                lresult_override = Some(self.handle_wm_paint(hwnd, wparam, lparam, window_id));
            }
            WM_NOTIFY => {
                (event_to_send, lresult_override) =
                    self._handle_wm_notify_dispatch(hwnd, wparam, lparam, window_id);
            }
            WM_APP_TREEVIEW_CHECKBOX_CLICKED => {
                event_to_send = treeview_handler::handle_wm_app_treeview_checkbox_clicked(
                    self, hwnd, window_id, wparam, lparam,
                );
            }
            WM_APP_MAIN_WINDOW_UI_SETUP_COMPLETE => {
                log::debug!(
                    "handle_window_message: Received message WM_APP_MAIN_WINDOW_UI_SETUP_COMPLETE"
                );
                event_to_send = Some(AppEvent::MainWindowUISetupComplete { window_id });
            }
            WM_APP_SPLITTER_DRAGGING | WM_APP_SPLITTER_DRAG_ENDED => {
                event_to_send = self.handle_wm_app_splitter(hwnd, wparam, lparam, window_id, msg);
            }
            WM_GETMINMAXINFO => {
                lresult_override =
                    Some(self.handle_wm_getminmaxinfo(hwnd, wparam, lparam, window_id));
            }
            WM_UAHDRAWMENU => {
                if self
                    .get_parsed_style(StyleId::MainWindowBackground)
                    .is_some()
                {
                    let uah = lparam.0 as *const UahMenu;
                    let colors = MenuBarColors::from_state(self);
                    unsafe { paint_dark_menu_bar(hwnd, (*uah).hdc, colors.bar_bg) };
                    lresult_override = Some(LRESULT(0));
                }
            }
            WM_UAHDRAWMENUITEM => {
                if self
                    .get_parsed_style(StyleId::MainWindowBackground)
                    .is_some()
                {
                    let udmi = unsafe { &*(lparam.0 as *const UahDrawMenuItem) };
                    let colors = MenuBarColors::from_state(self);
                    unsafe { paint_dark_menu_bar_item(udmi, &colors) };
                    lresult_override = Some(LRESULT(0));
                }
            }
            WM_NCPAINT | WM_NCACTIVATE => {
                if self
                    .get_parsed_style(StyleId::MainWindowBackground)
                    .is_some()
                {
                    let def_result = unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                    let colors = MenuBarColors::from_state(self);
                    unsafe { draw_dark_menu_nc_bottom_line(hwnd, colors.bar_bg) };
                    lresult_override = Some(def_result);
                }
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
                let hdc = HDC(wparam.0 as *mut c_void);
                let hwnd_control = HWND(lparam.0 as *mut c_void);
                let route = self.resolve_ctlcolor_route(window_id, hwnd_control, msg);
                lresult_override = match route {
                    paint_router::PaintRoute::Edit => {
                        input_handler::handle_wm_ctlcoloredit(self, window_id, hdc, hwnd_control)
                    }
                    paint_router::PaintRoute::LabelStatic => {
                        label_handler::handle_wm_ctlcolorstatic(self, window_id, hdc, hwnd_control)
                    }
                    paint_router::PaintRoute::ComboListBox => {
                        self.handle_wm_ctlcolorlistbox(window_id, hdc, hwnd_control)
                    }
                    paint_router::PaintRoute::Button => {
                        button_handler::handle_wm_ctlcolorbtn(self, window_id, hdc, hwnd_control)
                    }
                    _ => None,
                };
            }
            _ => {}
        }

        if let Some(event) = event_to_send {
            self.send_event(event);
        }

        if let Some(lresult) = lresult_override {
            lresult
        } else {
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }

    /*
     * Dispatches WM_NOTIFY messages to appropriate control handlers.
     * It inspects the NMHDR code to determine the type of notification and the
     * control that sent it, then routes to specific handlers (e.g., for TreeView
     * custom draw or general notifications).
     */
    fn _handle_wm_notify_dispatch(
        self: &Arc<Self>,
        hwnd_parent_window: HWND,
        _wparam_original: WPARAM,
        lparam_original: LPARAM,
        window_id: WindowId,
    ) -> (Option<AppEvent>, Option<LRESULT>) {
        let nmhdr_ptr = lparam_original.0 as *const NMHDR;
        if nmhdr_ptr.is_null() {
            log::warn!("WM_NOTIFY received with null NMHDR pointer. Ignoring.");
            return (None, None);
        }
        let nmhdr = unsafe { &*nmhdr_ptr };
        let control_id_from_notify = ControlId::new(nmhdr.idFrom as i32);

        let is_treeview_notification = self.with_window_data_read(window_id, |window_data| {
            Ok(window_data.has_treeview_state()
                && window_data.get_control_hwnd(control_id_from_notify) == Some(nmhdr.hwndFrom))
        });

        if let Ok(true) = is_treeview_notification {
            match nmhdr.code {
                NM_CUSTOMDRAW => {
                    log::trace!(
                        "Routing NM_CUSTOMDRAW from ControlID {} to treeview_handler.",
                        control_id_from_notify.raw()
                    );
                    let lresult = treeview_handler::handle_nm_customdraw(
                        self,
                        window_id,
                        lparam_original,
                        control_id_from_notify,
                    );
                    return (None, Some(lresult));
                }
                NM_CLICK => {
                    log::trace!(
                        "Routing NM_CLICK from ControlID {} to treeview_handler.",
                        control_id_from_notify.raw()
                    );
                    let event = treeview_handler::handle_nm_click(
                        self,
                        hwnd_parent_window,
                        window_id,
                        nmhdr,
                    );
                    return (event, None);
                }
                TVN_ITEMCHANGEDW => {
                    log::trace!(
                        "Routing TVN_ITEMCHANGEDW from ControlID {} to treeview_handler.",
                        control_id_from_notify.raw()
                    );
                    let event = treeview_handler::handle_treeview_itemchanged_notification(
                        self,
                        window_id,
                        lparam_original,
                        control_id_from_notify,
                    );
                    return (event, None);
                }
                _ => {
                    log::trace!(
                        "Unhandled WM_NOTIFY code {} from known TreeView ControlID {}.",
                        nmhdr.code,
                        control_id_from_notify.raw()
                    );
                }
            }
        } else if let Err(e) = is_treeview_notification {
            log::error!(
                "Failed to access window data for WM_NOTIFY in WinID {:?}: {:?}",
                window_id,
                e
            );
        }
        (None, None)
    }

    /*
     * Handles the WM_CREATE message for a window.
     * Ensures window-wide resources like custom fonts are created.
     */
    fn handle_wm_create(
        self: &Arc<Self>,
        hwnd: HWND,
        _wparam: WPARAM,
        _lparam: LPARAM,
        window_id: WindowId,
    ) {
        log::debug!("Platform: WM_CREATE for HWND {hwnd:?}, WindowId {window_id:?}");
        if let Err(e) = self.with_window_data_write(window_id, |window_data| {
            window_data.ensure_status_bar_font();
            window_data.ensure_treeview_new_item_font();
            Ok(())
        }) {
            log::error!(
                "Failed to access window data during WM_CREATE for WinID {window_id:?}: {e:?}"
            );
        }
        if self
            .get_parsed_style(StyleId::MainWindowBackground)
            .is_some()
        {
            try_enable_dark_mode(hwnd);
        }
    }

    /*
     * Triggers layout recalculation for the specified window.
     */
    pub(crate) fn trigger_layout_recalculation(self: &Arc<Self>, window_id: WindowId) {
        log::debug!("trigger_layout_recalculation called for WinID {window_id:?}");

        if let Err(e) = self.with_window_data_read(window_id, |window_data| {
            window_data.recalculate_and_apply_layout();
            Ok(())
        }) {
            log::error!(
                "Failed to access window data for layout recalculation of WinID {window_id:?}: {e:?}"
            );
        }
    }

    /*
     * Handles WM_SIZE: Triggers layout recalculation.
     */
    fn handle_wm_size(
        self: &Arc<Self>,
        hwnd: HWND,
        _wparam: WPARAM,
        width_height: LPARAM,
        window_id: WindowId,
    ) -> Option<AppEvent> {
        let client_width = loword_from_lparam(width_height);
        let client_height = hiword_from_lparam(width_height);
        log::debug!(
            "Platform: WM_SIZE for WinID {window_id:?}, HWND {hwnd:?}. Client: {client_width}x{client_height}"
        );
        self.trigger_layout_recalculation(window_id);
        Some(AppEvent::WindowResized {
            window_id,
            width: client_width,
            height: client_height,
        })
    }

    /*
     * Handles WM_COMMAND: Dispatches menu actions or button clicks.
     */
    fn handle_wm_command(
        self: &Arc<Self>,
        _hwnd_parent: HWND,
        wparam: WPARAM,
        control_hwnd: LPARAM,
        window_id: WindowId,
    ) -> Option<AppEvent> {
        let command_id = loword_from_wparam(wparam);
        let notification_code = highord_from_wparam(wparam);
        if control_hwnd.0 == 0 {
            return super::controls::menu_handler::handle_wm_command_for_menu(
                window_id,
                command_id,
                _hwnd_parent,
                self,
            );
        } else {
            // Control notification
            let hwnd_control = HWND(control_hwnd.0 as *mut std::ffi::c_void);
            let control_id = ControlId::new(command_id);

            // Route based on notification code and control kind
            if notification_code == BN_CLICKED as i32 {
                // Disambiguate between push buttons and radio buttons
                let kind_result = self.with_window_data_read(window_id, |window_data| {
                    Ok(window_data.get_control_kind(control_id))
                });

                match kind_result {
                    Ok(Some(ControlKind::RadioButton)) => {
                        log::debug!(
                            "RadioButton ID {} clicked in WinID {:?}",
                            control_id.raw(),
                            window_id
                        );
                        return Some(AppEvent::RadioButtonSelected {
                            window_id,
                            control_id,
                        });
                    }
                    Ok(Some(ControlKind::CheckBox)) => {
                        // BS_AUTOCHECKBOX has already toggled the state; read it back.
                        let checked = checkbox_handler::read_checkbox_state(hwnd_control);
                        log::debug!(
                            "CheckBox ID {} clicked in WinID {:?}, new checked={}",
                            control_id.raw(),
                            window_id,
                            checked
                        );
                        return Some(AppEvent::CheckBoxToggled {
                            window_id,
                            control_id,
                            checked,
                        });
                    }
                    Ok(Some(ControlKind::Button)) | Ok(None) => {
                        // Fallback to push button behavior
                        return Some(button_handler::handle_bn_clicked(
                            window_id,
                            control_id,
                            hwnd_control,
                        ));
                    }
                    Ok(Some(_)) => {
                        log::trace!(
                            "BN_CLICKED for non-button control ID {} in WinID {:?}",
                            control_id.raw(),
                            window_id
                        );
                    }
                    Err(err) => {
                        log::warn!(
                            "Failed to resolve ControlKind for BN_CLICKED on ID {} in WinID {:?}: {err:?}",
                            control_id.raw(),
                            window_id
                        );
                    }
                }
            } else if notification_code == CBN_SELCHANGE as i32 {
                // ComboBox selection changed
                let kind_result = self.with_window_data_read(window_id, |window_data| {
                    Ok(window_data.get_control_kind(control_id))
                });

                match kind_result {
                    Ok(Some(ControlKind::ComboBox)) => {
                        log::debug!(
                            "ComboBox ID {} selection changed in WinID {:?}",
                            control_id.raw(),
                            window_id
                        );
                        return Some(combobox_handler::handle_cbn_selchange(
                            window_id,
                            control_id,
                            hwnd_control,
                        ));
                    }
                    Ok(Some(_)) => {
                        log::trace!(
                            "CBN_SELCHANGE for non-combobox control ID {} in WinID {:?}",
                            control_id.raw(),
                            window_id
                        );
                    }
                    Ok(None) => {
                        log::warn!(
                            "CBN_SELCHANGE for unknown control ID {} in WinID {:?}",
                            control_id.raw(),
                            window_id
                        );
                    }
                    Err(err) => {
                        log::warn!(
                            "Failed to resolve ControlKind for CBN_SELCHANGE on ID {} in WinID {:?}: {err:?}",
                            control_id.raw(),
                            window_id
                        );
                    }
                }
            } else if notification_code == CBN_DROPDOWN as i32 {
                log::info!(
                    "ComboBox ID {} dropdown opened in WinID {:?}",
                    control_id.raw(),
                    window_id
                );
                let should_attempt_heal = self
                    .with_window_data_write(window_id, |window_data| {
                        Ok(window_data.mark_dropdown_heal_attempted(control_id))
                    })
                    .unwrap_or(false);
                if should_attempt_heal {
                    combobox_handler::validate_and_heal_dropdown_geometry(
                        window_id,
                        control_id,
                        hwnd_control,
                    );
                }
            } else if notification_code == CBN_CLOSEUP as i32 {
                log::info!(
                    "ComboBox ID {} dropdown closed in WinID {:?}",
                    control_id.raw(),
                    window_id
                );
                let _ = self.with_window_data_write(window_id, |window_data| {
                    window_data.clear_dropdown_heal_attempted(control_id);
                    Ok(())
                });
            } else if notification_code == EN_CHANGE as i32 {
                log::trace!(
                    "Edit control ID {} changed, starting debounce timer",
                    control_id.raw()
                );
                unsafe {
                    SetTimer(
                        Some(_hwnd_parent),
                        control_id.raw() as usize,
                        INPUT_DEBOUNCE_MS,
                        None,
                    );
                }
            } else if notification_code == EN_VSCROLL as i32 {
                return self.handle_edit_control_scroll(window_id, control_id, hwnd_control);
            } else {
                log::trace!(
                    "Unhandled WM_COMMAND from control: ID {}, NotifyCode {notification_code}, HWND {hwnd_control:?}, WinID {window_id:?}",
                    control_id.raw()
                );
            }
        }
        None
    }

    /*
     * Handles scrollbar notifications for EDIT controls and forwards real user scrolls to app logic.
     * Suppressed controls skip event generation so linked scrolling commands don't echo back.
     */
    fn handle_edit_control_scroll(
        self: &Arc<Self>,
        window_id: WindowId,
        control_id: ControlId,
        hwnd_control: HWND,
    ) -> Option<AppEvent> {
        if is_scroll_event_suppressed(window_id, control_id) {
            log::info!(
                "[Scroll] Suppressed programmatic scroll for ControlID {} in WinID {:?}",
                control_id.raw(),
                window_id
            );
            return None;
        }

        let vertical = match query_scroll_percentage(hwnd_control, SB_VERT) {
            Some(value) => value,
            None => {
                log::info!(
                    "[Scroll] No vertical scroll info for ControlID {} in WinID {:?} (HWND {:?})",
                    control_id.raw(),
                    window_id,
                    hwnd_control
                );
                return None;
            }
        };
        let horizontal = query_scroll_percentage(hwnd_control, SB_HORZ).unwrap_or(0);
        Some(AppEvent::ControlScrolled {
            window_id,
            control_id,
            vertical_pos: vertical,
            horizontal_pos: horizontal,
        })
    }

    fn handle_wm_timer(
        self: &Arc<Self>,
        hwnd: HWND,
        timer_id: WPARAM,
        _lparam: LPARAM,
        window_id: WindowId,
    ) -> Option<AppEvent> {
        unsafe {
            _ = KillTimer(Some(hwnd), timer_id.0);
        }
        let control_id = ControlId::new(timer_id.0 as i32);

        let hwnd_edit_result = self.with_window_data_read(window_id, |window_data| {
            window_data.get_control_hwnd(control_id).ok_or_else(|| {
                log::warn!("Control not found for timer ID {}", control_id.raw());
                PlatformError::InvalidHandle("Control not found for timer".into())
            })
        });

        if let Ok(hwnd_edit) = hwnd_edit_result {
            match read_edit_control_text(hwnd_edit) {
                Ok(text) => {
                    return Some(AppEvent::InputTextChanged {
                        window_id,
                        control_id,
                        text,
                    });
                }
                Err(err) => {
                    log::error!(
                        "Failed to read text for control {} in window {:?}: {err}",
                        control_id.raw(),
                        window_id
                    );
                }
            }
        }
        None
    }

    /*
     * Handles WM_APP_SPLITTER_DRAGGING and WM_APP_SPLITTER_DRAG_ENDED messages.
     * These are sent by the splitter control's window procedure during drag operations.
     */
    fn handle_wm_app_splitter(
        self: &Arc<Self>,
        _hwnd_parent: HWND,
        wparam: WPARAM,
        lparam: LPARAM,
        window_id: WindowId,
        msg: u32,
    ) -> Option<AppEvent> {
        // WPARAM contains the splitter's HWND
        let hwnd_splitter = HWND(wparam.0 as *mut std::ffi::c_void);
        // LPARAM contains the desired_left_width_px
        let desired_left_width_px = lparam.0 as i32;

        // Get the control ID from the splitter's HWND
        let control_id_raw = unsafe { GetDlgCtrlID(hwnd_splitter) };
        if control_id_raw == 0 {
            log::warn!(
                "Splitter message for HWND {:?} without control ID",
                hwnd_splitter
            );
            return None;
        }

        let control_id = ControlId::new(control_id_raw);

        match msg {
            WM_APP_SPLITTER_DRAGGING => {
                let _ = self.with_window_data_write(window_id, |window_data| {
                    window_data.set_suppress_erasebkgnd(true);
                    Ok(())
                });
                log::trace!(
                    "SplitterHandler: Dragging splitter ID {} - desired_left_width_px: {}",
                    control_id.raw(),
                    desired_left_width_px
                );
                Some(AppEvent::SplitterDragging {
                    window_id,
                    control_id,
                    desired_left_width_px,
                })
            }
            WM_APP_SPLITTER_DRAG_ENDED => {
                let _ = self.with_window_data_write(window_id, |window_data| {
                    window_data.set_suppress_erasebkgnd(false);
                    Ok(())
                });
                log::debug!(
                    "SplitterHandler: Drag ended for splitter ID {} - final desired_left_width_px: {}",
                    control_id.raw(),
                    desired_left_width_px
                );
                Some(AppEvent::SplitterDragEnded {
                    window_id,
                    control_id,
                    desired_left_width_px,
                })
            }
            _ => None,
        }
    }

    fn resolve_ctlcolor_route(
        self: &Arc<Self>,
        window_id: WindowId,
        hwnd_control: HWND,
        msg: u32,
    ) -> paint_router::PaintRoute {
        let control_id_raw = unsafe { GetDlgCtrlID(hwnd_control) };
        if control_id_raw == 0 {
            if self.is_combo_owned_ctlcolor_hwnd(window_id, hwnd_control) {
                return paint_router::PaintRoute::ComboListBox;
            }
            warn!(
                "[Paint] WM_CTLCOLOR message for HWND {:?} without control ID; using fallback",
                hwnd_control
            );
            return fallback_ctlcolor_route(msg);
        }

        let control_id = ControlId::new(control_id_raw);
        let kind_result = self.with_window_data_read(window_id, |window_data| {
            Ok(window_data.get_control_kind(control_id))
        });

        match kind_result {
            Ok(Some(kind)) => paint_router::resolve_paint_route(kind, msg),
            Ok(None) => {
                if self.is_combo_owned_ctlcolor_hwnd(window_id, hwnd_control) {
                    return paint_router::PaintRoute::ComboListBox;
                }
                // For WM_CTLCOLORLISTBOX, the HWND is the dropdown listbox
                // auto-created by Windows inside a CBS_DROPDOWNLIST combo box.
                // It carries a Windows-assigned control ID (typically 1000)
                // that we never register, so normal lookup misses. Route
                // directly to ComboListBox — this message is only ever sent
                // by combo box controls.
                if msg == WM_CTLCOLORLISTBOX {
                    return paint_router::PaintRoute::ComboListBox;
                }
                warn!(
                    "[Paint] Missing ControlKind for ControlID {} in WinID {:?}; using fallback",
                    control_id.raw(),
                    window_id
                );
                fallback_ctlcolor_route(msg)
            }
            Err(err) => {
                warn!(
                    "[Paint] Failed to resolve ControlKind for ControlID {} in WinID {:?}: {err:?}",
                    control_id.raw(),
                    window_id
                );
                fallback_ctlcolor_route(msg)
            }
        }
    }

    fn is_combo_owned_ctlcolor_hwnd(
        self: &Arc<Self>,
        window_id: WindowId,
        hwnd_control: HWND,
    ) -> bool {
        let hwnd_parent = match unsafe { GetParent(hwnd_control) } {
            Ok(hwnd) if !hwnd.is_invalid() && hwnd != HWND_INVALID => hwnd,
            _ => return false,
        };

        let is_combo_owned = self
            .with_window_data_read(window_id, |window_data| {
                let owner_control_id = window_data.find_control_id_by_hwnd(hwnd_parent);
                Ok(
                    owner_control_id
                        .and_then(|control_id| window_data.get_control_kind(control_id))
                        == Some(ControlKind::ComboBox),
                )
            })
            .unwrap_or(false);

        if is_combo_owned {
            log::trace!(
                "[Paint] WM_CTLCOLOR HWND {:?} resolved as combo-owned child (parent={:?})",
                hwnd_control,
                hwnd_parent
            );
        }

        is_combo_owned
    }

    /*
     * Handles WM_CTLCOLORLISTBOX for ComboBox dropdown lists.
     * Applies dark theme styling by resolving ComboBox or DefaultInput styles.
     */
    fn handle_wm_ctlcolorlistbox(
        self: &Arc<Self>,
        window_id: WindowId,
        hdc: HDC,
        hwnd_combo_child: HWND,
    ) -> Option<LRESULT> {
        log::trace!(
            "[Paint] handle_wm_ctlcolorlistbox for combo child HWND {:?} in WinID {:?}",
            hwnd_combo_child,
            window_id
        );

        // Best effort: apply dark theme one time per child/owner HWND pair.
        // Reapplying SetWindowTheme during every WM_CTLCOLOR can cause excessive
        // repaint churn and effectively self-sustaining paint traffic.
        if Self::mark_combo_hwnd_themed(hwnd_combo_child) {
            unsafe {
                let _ = SetWindowTheme(hwnd_combo_child, w!("DarkMode_Explorer"), None);
                try_enable_dark_mode(hwnd_combo_child);
            }
        }

        // Resolve style: try ComboBox first, fallback to DefaultInput
        let style = self
            .get_parsed_style(StyleId::ComboBox)
            .or_else(|| self.get_parsed_style(StyleId::DefaultInput));

        if let Some(style) = style {
            log::trace!(
                "[Paint] Applying ComboBox dark colors for HWND {:?} in WinID {:?}",
                hwnd_combo_child,
                window_id
            );
            // Convert Color to COLORREF using styling_handler helper
            if let Some(ref text_color) = style.text_color {
                unsafe {
                    SetTextColor(hdc, styling_handler::color_to_colorref(text_color));
                }
            }
            if let Some(ref bg_color) = style.background_color {
                unsafe {
                    SetBkColor(hdc, styling_handler::color_to_colorref(bg_color));
                }
            }
            if let Some(bg_brush) = style.background_brush {
                return Some(LRESULT(bg_brush.0 as isize));
            }
        }

        None
    }

    fn mark_combo_hwnd_themed(hwnd: HWND) -> bool {
        static THEMED_HWNDS: OnceLock<Mutex<HashSet<isize>>> = OnceLock::new();
        let set = THEMED_HWNDS.get_or_init(|| Mutex::new(HashSet::new()));
        match set.lock() {
            Ok(mut guard) => guard.insert(hwnd.0 as isize),
            Err(_) => false,
        }
    }

    fn handle_wm_erasebkgnd(
        self: &Arc<Self>,
        hwnd: HWND,
        wparam: WPARAM,
        lparam: LPARAM,
        _window_id: WindowId,
    ) -> LRESULT {
        log::debug!("[Paint] WM_ERASEBKGND hwnd={hwnd:?}");
        let suppress = self
            .with_window_data_read(_window_id, |window_data| {
                Ok(window_data.suppresses_erasebkgnd())
            })
            .unwrap_or(false);
        if suppress {
            return LRESULT(1);
        }
        unsafe {
            if let Some(style) = self.get_parsed_style(StyleId::MainWindowBackground)
                && let Some(bg_brush) = style.background_brush
            {
                let hdc = HDC(wparam.0 as *mut c_void);
                let mut client_rect = RECT::default();
                if GetClientRect(hwnd, &mut client_rect).is_ok() {
                    FillRect(hdc, &client_rect, bg_brush);
                    return LRESULT(1);
                }
            }
            DefWindowProcW(hwnd, WM_ERASEBKGND, wparam, lparam)
        }
    }

    /*
     * Handles WM_PAINT: Fills background. Control custom drawing is separate.
     */
    fn handle_wm_paint(
        self: &Arc<Self>,
        hwnd: HWND,
        _wparam: WPARAM,
        _lparam: LPARAM,
        _window_id: WindowId,
    ) -> LRESULT {
        log::debug!("[Paint] WM_PAINT hwnd={hwnd:?}");
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if !hdc.is_invalid() {
                let background_brush = self
                    .get_parsed_style(StyleId::MainWindowBackground)
                    .and_then(|style| style.background_brush)
                    .unwrap_or(HBRUSH((COLOR_WINDOW.0 + 1) as *mut c_void));

                FillRect(hdc, &ps.rcPaint, background_brush);
                _ = EndPaint(hwnd, &ps);
            }
        }
        SUCCESS_CODE
    }

    /*
     * Handles WM_GETMINMAXINFO: Sets minimum window tracking size.
     */
    fn handle_wm_getminmaxinfo(
        self: &Arc<Self>,
        _hwnd: HWND,
        _wparam: WPARAM,
        lparam: LPARAM,
        _window_id: WindowId,
    ) -> LRESULT {
        if lparam.0 != 0 {
            let mmi = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            mmi.ptMinTrackSize.x = 300;
            mmi.ptMinTrackSize.y = 200;
        }
        SUCCESS_CODE
    }
}

fn fallback_ctlcolor_route(msg: u32) -> paint_router::PaintRoute {
    match msg {
        WM_CTLCOLOREDIT => paint_router::PaintRoute::Edit,
        WM_CTLCOLORSTATIC => paint_router::PaintRoute::LabelStatic,
        WM_CTLCOLORLISTBOX => paint_router::PaintRoute::ComboListBox,
        WM_CTLCOLORBTN => paint_router::PaintRoute::Button,
        _ => paint_router::PaintRoute::Default,
    }
}

/*
 * Reads the requested scrollbar and converts its position into a 0-100 percentage.
 * Falls back to `None` when the underlying control does not expose scroll metadata.
 */
fn query_scroll_percentage(hwnd: HWND, bar: SCROLLBAR_CONSTANTS) -> Option<u32> {
    let mut scroll_info = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        ..Default::default()
    };

    if let Err(err) = unsafe { GetScrollInfo(hwnd, bar, &mut scroll_info) } {
        log::trace!(
            "query_scroll_percentage: GetScrollInfo failed for bar {:?} on {:?}: {err:?}",
            bar,
            hwnd
        );
        return None;
    }

    let min = scroll_info.nMin as i64;
    let max = scroll_info.nMax as i64;
    let mut range = max - min;
    if scroll_info.nPage > 0 && range > 0 {
        range -= (scroll_info.nPage as i64 - 1).max(0);
    }

    if range <= 0 {
        return Some(0);
    }

    let pos = (scroll_info.nPos as i64 - min).max(0).min(range);
    let percent = ((pos * 100) / range).clamp(0, 100) as u32;
    Some(percent)
}

// Reads the full contents of an EDIT control without truncation.
pub(crate) fn read_edit_control_text(hwnd_edit: HWND) -> PlatformResult<String> {
    read_edit_control_text_with(
        || unsafe { GetWindowTextLengthW(hwnd_edit) },
        |buf| unsafe { GetWindowTextW(hwnd_edit, buf) },
    )
}

// Internal helper that can be unit tested with injected getters.
fn read_edit_control_text_with<FLen, FGet>(get_len: FLen, get_text: FGet) -> PlatformResult<String>
where
    FLen: Fn() -> i32,
    FGet: Fn(&mut [u16]) -> i32,
{
    let len = get_len();
    if len < 0 {
        return Err(PlatformError::OperationFailed(
            "GetWindowTextLengthW returned negative length".into(),
        ));
    }

    let mut buffer = vec![0u16; len as usize + 1];
    let copied = get_text(&mut buffer);
    if copied < 0 {
        return Err(PlatformError::OperationFailed(
            "GetWindowTextW returned negative length".into(),
        ));
    }

    let copied = copied as usize;
    buffer.truncate(copied);
    Ok(String::from_utf16_lossy(&buffer))
}

pub(crate) fn set_window_title(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    title: &str,
) -> PlatformResult<()> {
    log::debug!("Setting title for WinID {window_id:?} to '{title}'");
    internal_state.with_window_data_read(window_id, |window_data| {
        let hwnd = window_data.get_hwnd();
        if hwnd.is_invalid() {
            return Err(PlatformError::InvalidHandle(format!(
                "HWND for WinID {window_id:?} is invalid in set_window_title"
            )));
        }
        unsafe { SetWindowTextW(hwnd, &HSTRING::from(title))? };
        Ok(())
    })
}

pub(crate) fn show_window(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    show: bool,
) -> PlatformResult<()> {
    log::debug!("Setting visibility for WinID {window_id:?} to {show}");
    internal_state.with_window_data_read(window_id, |window_data| {
        let hwnd = window_data.get_hwnd();
        if hwnd.is_invalid() {
            return Err(PlatformError::InvalidHandle(format!(
                "HWND for WinID {window_id:?} is invalid in show_window"
            )));
        }
        let cmd = if show { SW_SHOW } else { SW_HIDE };
        unsafe { _ = ShowWindow(hwnd, cmd) };
        Ok(())
    })
}

/*
 * Initiates the closing of a specified window by calling DestroyWindow directly.
 * The actual destruction sequence (WM_DESTROY, etc.) will follow.
 */
pub(crate) fn send_close_message(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
) -> PlatformResult<()> {
    log::debug!(
        "Platform: send_close_message received for WindowId {window_id:?}, attempting to destroy native window directly."
    );
    // This function will get the HWND and call DestroyWindow.
    // If successful, WM_DESTROY will be posted to the window's queue,
    // and the cleanup path in the window procedure will run.
    destroy_native_window(internal_state, window_id)
}

/*
 * Attempts to destroy the native window associated with the given `WindowId`.
 * This is called by `send_close_message` or can be used for more direct cleanup.
 */
pub(crate) fn destroy_native_window(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
) -> PlatformResult<()> {
    log::debug!("Attempting to destroy native window for WinID {window_id:?}");

    let hwnd_to_destroy =
        internal_state.with_window_data_read(window_id, |window_data| Ok(window_data.get_hwnd()));

    match hwnd_to_destroy {
        Ok(hwnd) if !hwnd.is_invalid() => {
            log::debug!("Calling DestroyWindow for HWND {hwnd:?} (WinID {window_id:?})");
            unsafe {
                if DestroyWindow(hwnd).is_err() {
                    let last_error = GetLastError();
                    // Don't error out if the handle is already invalid (e.g., already destroyed).
                    if last_error.0 != ERROR_INVALID_WINDOW_HANDLE.0 {
                        log::error!("DestroyWindow for HWND {hwnd:?} failed: {last_error:?}");
                    } else {
                        log::debug!(
                            "DestroyWindow for HWND {hwnd:?} reported invalid handle (already destroyed?)."
                        );
                    }
                } else {
                    log::debug!(
                        "DestroyWindow call initiated for HWND {hwnd:?}. WM_DESTROY will follow."
                    );
                }
            }
        }
        Ok(_) => {
            // HWND is invalid
            log::warn!("HWND for WinID {window_id:?} was invalid before DestroyWindow call.");
        }
        Err(_) => {
            // WindowId not found
            log::warn!("WinID {window_id:?} not found for destroy_native_window.");
        }
    };
    // This function's purpose is to *try* to destroy, so don't bubble up "not found" as an error.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::HWND;

    /*
     * Unit tests for NativeWindowData. These tests verify basic state
     * management without invoking Win32 APIs, using dummy HWND values.
     */

    #[test]
    // [CDU-ControlLogicalIdsV1][CDU-ControlEnableDisableV1][CDU-Tech-WindowsRsV1]
    // Control registration via logical IDs using windows-rs HWNDs powers later enable/disable commands.
    fn test_register_control_hwnd_lookup() {
        // Arrange
        let mut data = NativeWindowData::new(WindowId(1));
        let hwnd = HWND(0x1234 as *mut std::ffi::c_void);
        // Act
        let control_id = ControlId::new(42);
        data.register_control_hwnd(control_id, hwnd);
        // Assert
        assert_eq!(data.get_control_hwnd(control_id), Some(hwnd));
        assert!(data.has_control(control_id));
    }

    #[test]
    // [CDU-CmdEventPatternV1] Menu action lookup tables make WM_COMMAND notifications translatable to AppEvents.
    fn test_register_menu_action_increments_counter() {
        // Arrange
        let mut data = NativeWindowData::new(WindowId(2));
        let start = data.get_next_menu_item_id_counter();
        // Act
        let id1 = data.register_menu_action(MenuActionId(42));
        let id2 = data.register_menu_action(MenuActionId(42));
        // Assert
        assert_eq!(data.menu_action_count(), 2);
        assert_eq!(id1, start);
        assert_eq!(id2, start + 1);
        assert_eq!(data.get_next_menu_item_id_counter(), start + 2);
        assert_eq!(data.get_menu_action(id1), Some(MenuActionId(42)));
    }

    #[test]
    // [CDU-Control-LabelV1][CDU-ControlTextUpdateV1] Label metadata keeps severity alongside text updates.
    fn test_set_and_get_label_severity() {
        // Arrange
        let mut data = NativeWindowData::new(WindowId(3));
        // Act
        let label_id = ControlId::new(7);
        data.set_label_severity(label_id, MessageSeverity::Warning);
        // Assert
        assert_eq!(
            data.get_label_severity(label_id),
            Some(MessageSeverity::Warning)
        );
    }

    #[test]
    // [CDU-Styling-ApplyV1] Applying a style pins the selected `StyleId` so redraw hooks can query it later.
    fn test_apply_style_to_control_records_id() {
        let mut data = NativeWindowData::new(WindowId(4));
        let control_id = ControlId::new(8);
        data.apply_style_to_control(control_id, StyleId::DefaultText);
        assert_eq!(
            data.get_style_for_control(control_id),
            Some(StyleId::DefaultText)
        );
    }

    #[test]
    fn read_edit_control_text_with_handles_strings_longer_than_default_buffer() {
        let long_text = "https://example.com/".repeat(20); // 380 chars
        let utf16: Vec<u16> = long_text.encode_utf16().collect();
        let expected_len = utf16.len() as i32;

        let result = read_edit_control_text_with(
            || expected_len,
            |buf| {
                buf[..utf16.len()].copy_from_slice(&utf16);
                utf16.len() as i32
            },
        )
        .expect("should read text without truncation");

        assert_eq!(result, long_text);
    }

    /*
     * Unit tests for the pure layout calculation. These tests ensure the
     * geometry is computed correctly without creating any native windows.
     */

    #[test]
    // [CDU-LayoutSystemV1] Docking rules produce deterministic rectangles even without native HWNDs.
    fn test_calculate_layout_top_and_fill() {
        // Arrange
        let rules = vec![
            LayoutRule {
                control_id: ControlId::new(1),
                parent_control_id: None,
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(20),
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: ControlId::new(2),
                parent_control_id: None,
                dock_style: DockStyle::Fill,
                order: 1,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
        ];
        let parent_rect = RECT {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        };
        // Act
        let map = NativeWindowData::calculate_layout(parent_rect, &rules);
        // Assert
        assert_eq!(map.get(&ControlId::new(1)).unwrap().bottom, 20);
        assert_eq!(map.get(&ControlId::new(2)).unwrap().top, 20);
        assert_eq!(map.get(&ControlId::new(2)).unwrap().bottom, 100);
    }

    #[test]
    fn define_layout_validation_rejects_multiple_fill_siblings() {
        let rules = vec![
            LayoutRule {
                control_id: ControlId::new(10),
                parent_control_id: Some(ControlId::new(1)),
                dock_style: DockStyle::Fill,
                order: 0,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: ControlId::new(11),
                parent_control_id: Some(ControlId::new(1)),
                dock_style: DockStyle::Fill,
                order: 1,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
        ];

        let err = NativeWindowData::validate_layout_rules(&rules)
            .expect_err("multiple Fill siblings should be rejected");
        let message = err.to_string();
        assert!(message.contains("multiple DockStyle::Fill children"));
        assert!(message.contains("10"));
        assert!(message.contains("11"));
    }

    #[test]
    fn define_layout_validation_allows_one_fill_per_parent() {
        let rules = vec![
            LayoutRule {
                control_id: ControlId::new(20),
                parent_control_id: Some(ControlId::new(1)),
                dock_style: DockStyle::Fill,
                order: 0,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: ControlId::new(21),
                parent_control_id: Some(ControlId::new(2)),
                dock_style: DockStyle::Fill,
                order: 0,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: ControlId::new(22),
                parent_control_id: Some(ControlId::new(1)),
                dock_style: DockStyle::Top,
                order: 1,
                fixed_size: Some(10),
                margin: (0, 0, 0, 0),
            },
        ];

        NativeWindowData::validate_layout_rules(&rules)
            .expect("one Fill child per parent should be valid");
    }

    #[test]
    // [CDU-LayoutSystemV1] Proportional fills divide available space using the declarative weights.
    fn test_calculate_layout_proportional_fill() {
        // Arrange
        let rules = vec![
            LayoutRule {
                control_id: ControlId::new(1),
                parent_control_id: None,
                dock_style: DockStyle::ProportionalFill { weight: 1.0 },
                order: 0,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: ControlId::new(2),
                parent_control_id: None,
                dock_style: DockStyle::ProportionalFill { weight: 2.0 },
                order: 1,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
        ];
        let parent_rect = RECT {
            left: 0,
            top: 0,
            right: 100,
            bottom: 20,
        };
        // Act
        let map = NativeWindowData::calculate_layout(parent_rect, &rules);
        // Assert
        let rect1 = map.get(&ControlId::new(1)).unwrap();
        let rect2 = map.get(&ControlId::new(2)).unwrap();
        assert_eq!(rect1.right - rect1.left, 33);
        assert_eq!(rect2.left, 33);
        assert_eq!(rect2.right - rect2.left, 66);
    }

    #[test]
    // [CDU-LayoutSystemV1][CDU-Control-PanelV1] Nested panel layouts respect parent-child docking relationships.
    fn test_calculate_layout_nested_panels() {
        // Arrange
        let outer_rule = LayoutRule {
            control_id: ControlId::new(1),
            parent_control_id: None,
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (0, 0, 0, 0),
        };
        let inner_rules = vec![
            LayoutRule {
                control_id: ControlId::new(2),
                parent_control_id: Some(ControlId::new(1)),
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(10),
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: ControlId::new(3),
                parent_control_id: Some(ControlId::new(1)),
                dock_style: DockStyle::Fill,
                order: 1,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
        ];
        let parent_rect = RECT {
            left: 0,
            top: 0,
            right: 50,
            bottom: 50,
        };
        // Act
        let outer_map =
            NativeWindowData::calculate_layout(parent_rect, std::slice::from_ref(&outer_rule));
        let outer_rect = outer_map.get(&ControlId::new(1)).unwrap();
        let inner_map = NativeWindowData::calculate_layout(
            RECT {
                left: 0,
                top: 0,
                right: outer_rect.right - outer_rect.left,
                bottom: outer_rect.bottom - outer_rect.top,
            },
            &inner_rules,
        );
        // Assert
        assert_eq!(outer_rect.right - outer_rect.left, 50);
        assert_eq!(inner_map.get(&ControlId::new(2)).unwrap().bottom, 10);
        assert_eq!(inner_map.get(&ControlId::new(3)).unwrap().top, 10);
    }

    #[test]
    fn resolve_dark_mode_uxtheme_ordinals_detects_expected_ordinals() {
        let ordinals = resolve_dark_mode_uxtheme_ordinals(|ordinal| {
            matches!(
                ordinal,
                UXTHEME_ORD_ALLOW_DARK_MODE_FOR_WINDOW | UXTHEME_ORD_FLUSH_MENU_THEMES
            )
        });

        assert!(ordinals.allow_dark_mode_for_window);
        assert!(!ordinals.set_preferred_app_mode);
        assert!(ordinals.flush_menu_themes);
    }

    #[test]
    fn find_control_id_by_hwnd_returns_registered_control() {
        let mut data = NativeWindowData::new(WindowId::new(1));
        let control_id = ControlId::new(42);
        let hwnd = HWND(0x1234isize as *mut _);

        data.register_control_hwnd(control_id, hwnd);

        assert_eq!(data.find_control_id_by_hwnd(hwnd), Some(control_id));
    }

    #[test]
    fn effective_native_height_for_combobox_is_expanded() {
        let mut data = NativeWindowData::new(WindowId::new(1));
        let combo_id = ControlId::new(77);
        data.register_control_kind(combo_id, ControlKind::ComboBox);
        data.register_control_hwnd(combo_id, HWND(0x1234isize as *mut _));

        let result = data.effective_native_height_for_control(combo_id, 26);

        assert!(result >= combobox_handler::fallback_min_dropdown_height_px());
    }
}
