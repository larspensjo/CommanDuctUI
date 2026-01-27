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
    controls::{button_handler, input_handler, label_handler, paint_router, treeview_handler},
    error::{PlatformError, Result as PlatformResult},
    styling::StyleId,
    types::{AppEvent, ControlId, DockStyle, LayoutRule, MenuActionId, MessageSeverity, WindowId},
};

use windows::core::w;
use windows::{
    Win32::{
        Foundation::{
            ERROR_INVALID_WINDOW_HANDLE, GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM,
        },
        Graphics::Dwm::{DWMWINDOWATTRIBUTE, DwmSetWindowAttribute},
        Graphics::Gdi::{
            BeginPaint, CLIP_DEFAULT_PRECIS, COLOR_WINDOW, CreateFontIndirectW, CreateFontW,
            DEFAULT_CHARSET, DEFAULT_GUI_FONT, DEFAULT_QUALITY, DeleteObject, EndPaint,
            FF_DONTCARE, FW_BOLD, FW_NORMAL, FillRect, GetDC, GetDeviceCaps, GetObjectW,
            GetStockObject, HBRUSH, HDC, HFONT, HGDIOBJ, LOGFONTW, LOGPIXELSY, OUT_DEFAULT_PRECIS,
            PAINTSTRUCT, ReleaseDC,
        },
        System::WindowsProgramming::MulDiv,
        UI::Controls::{
            DRAWITEMSTRUCT, NM_CLICK, NM_CUSTOMDRAW, NMHDR, SetWindowTheme, TVN_ITEMCHANGEDW,
        },
        UI::WindowsAndMessaging::*, // This list is massive, just import all of them.
    },
    core::{HSTRING, PCWSTR},
};

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::Arc;

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

// General UI constants
/// Default debounce delay for edit controls in milliseconds.
pub const INPUT_DEBOUNCE_MS: u32 = 300;

// Represents an invalid HWND, useful for initialization or checks.
pub(crate) const HWND_INVALID: HWND = HWND(std::ptr::null_mut());

const SUCCESS_CODE: LRESULT = LRESULT(0);

/// Identifies the kind of a control so styles can be dispatched without Win32 class queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlKind {
    Button,
    ProgressBar,
    TreeView,
    Static,
    Edit,
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

        let layout_map = NativeWindowData::calculate_layout(parent_rect, &child_rules);

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
            unsafe {
                _ = MoveWindow(hwnd, rect.left, rect.top, width, height, true);
            }
            if all_window_rules
                .iter()
                .any(|r_child| r_child.parent_control_id == Some(rule.control_id))
            {
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

    pub(crate) fn define_layout(&mut self, rules: Vec<LayoutRule>) {
        self.layout_rules = Some(rules);
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
            WINDOW_EX_STYLE::default(),            // Optional extended window styles
            &class_name_hstring,                   // Window class name
            &HSTRING::from(title),                 // Window title
            WS_OVERLAPPEDWINDOW,                   // Common window style
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

/// Best-effort enablement of dark mode for non-client areas (notably scrollbars) on supported OS builds.
pub(crate) fn try_enable_dark_mode(hwnd: HWND) {
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
            WM_GETMINMAXINFO => {
                lresult_override =
                    Some(self.handle_wm_getminmaxinfo(hwnd, wparam, lparam, window_id));
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
                let hdc = HDC(wparam.0 as *mut c_void);
                let hwnd_control = HWND(lparam.0 as *mut c_void);
                let route = self.resolve_ctlcolor_route(window_id, hwnd_control, msg);
                lresult_override = match route {
                    paint_router::PaintRoute::Edit => {
                        input_handler::handle_wm_ctlcoloredit(self, window_id, hdc, hwnd_control)
                    }
                    paint_router::PaintRoute::LabelStatic => {
                        label_handler::handle_wm_ctlcolorstatic(
                            self,
                            window_id,
                            hdc,
                            hwnd_control,
                        )
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
            if notification_code == BN_CLICKED as i32 {
                return Some(button_handler::handle_bn_clicked(
                    window_id,
                    control_id,
                    hwnd_control,
                ));
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

    fn resolve_ctlcolor_route(
        self: &Arc<Self>,
        window_id: WindowId,
        hwnd_control: HWND,
        msg: u32,
    ) -> paint_router::PaintRoute {
        let control_id_raw = unsafe { GetDlgCtrlID(hwnd_control) };
        if control_id_raw == 0 {
            warn!(
                "[Paint] WM_CTLCOLOR message for HWND {:?} without control ID; using fallback",
                hwnd_control
            );
            return fallback_ctlcolor_route(msg);
        }

        let control_id = ControlId::new(control_id_raw);
        let kind_result =
            self.with_window_data_read(window_id, |window_data| Ok(window_data.get_control_kind(control_id)));

        match kind_result {
            Ok(Some(kind)) => paint_router::resolve_paint_route(kind, msg),
            Ok(None) => {
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

    fn handle_wm_erasebkgnd(
        self: &Arc<Self>,
        hwnd: HWND,
        wparam: WPARAM,
        lparam: LPARAM,
        _window_id: WindowId,
    ) -> LRESULT {
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
}
