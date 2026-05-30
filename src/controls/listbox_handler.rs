#![allow(unsafe_op_in_unsafe_fn)]

/*
 * Owner-drawn list control for structured rows.
 *
 * This is a purpose-built control used for the Harvester left pane. It paints
 * a fixed badge column, a title line, and a metadata line for each row, and it
 * forwards selection / scroll changes back to the app layer through custom
 * messages routed by `window_common`.
 */

use crate::app::Win32ApiInternalState;
use crate::controls::gdi_utils::SelectedObject;
use crate::controls::keyboard_navigation::{
    KeyboardNavigation, apply_window_style, dialog_code, focus_on_click,
};
use crate::controls::styling_handler::color_to_colorref;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::ffi_safety::{self, DeferredWindowState};
use crate::styling::{Color, ParsedControlStyle, StyleId};
use crate::types::{ControlId, ListBoxItemDescriptor, ListBoxItemId, ListBoxRowDensity, WindowId};
use crate::win32_cast::{i32_from_usize_saturating, u32_from_usize_saturating};
use crate::window_common::{
    ControlKind, WM_APP_LISTBOX_KEYDOWN, WM_APP_LISTBOX_SCROLLED, WM_APP_LISTBOX_SELECTION_CHANGED,
    get_x_lparam, get_y_lparam, try_enable_dark_mode,
};

use std::sync::{Arc, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DEFAULT_GUI_FONT, DeleteObject, DrawTextW, EndPaint, FillRect,
    GetStockObject, GetTextExtentPoint32W, HDC, HGDIOBJ, InvalidateRect, PAINTSTRUCT, RoundRect,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::SetScrollInfo;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_END, VK_HOME, VK_NEXT, VK_PRIOR, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, GET_ANCESTOR_FLAGS, GWLP_USERDATA,
    GetAncestor, GetClientRect, GetDlgCtrlID, GetWindowLongPtrW, HMENU, RegisterClassW, SB_BOTTOM,
    SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP,
    SB_VERT, SCROLLINFO, SIF_PAGE, SIF_POS, SIF_RANGE, SendMessageW, SetWindowLongPtrW,
    WINDOW_EX_STYLE, WM_ERASEBKGND, WM_GETDLGCODE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SIZE, WM_VSCROLL, WNDCLASSW, WS_CHILD,
    WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, PCWSTR, w};

// WM_MOUSELEAVE is not exported by windows-rs in this crate version.
const WM_MOUSELEAVE: u32 = 0x02A3;
const KEYBOARD_NAVIGATION: KeyboardNavigation = KeyboardNavigation::DIALOG_NAVIGATION;

const LIST_BOX_CLASS_NAME: PCWSTR = w!("CommanDuctUIOwnerDrawnListBox");
static LIST_BOX_CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

const EXPANDED_ROW_HEIGHT: i32 = 44;
const COMPACT_ROW_HEIGHT: i32 = 30;
const ROW_PAD_TOP: i32 = 6;
const ROW_PAD_LEFT: i32 = 12;
const ROW_PAD_RIGHT: i32 = 12;
const BADGE_PAD_X: i32 = 6;
const BADGE_GAP: i32 = 4;
const BADGE_HEIGHT: i32 = 16;
const BADGE_RADIUS: i32 = 3;
const ACCENT_WIDTH: i32 = 3;
const DEFAULT_BADGE_COLUMN_WIDTH: i32 = 130;

#[derive(Debug, Clone, Copy)]
struct ColorPair {
    background: Color,
    text: Color,
}

#[derive(Debug, Clone)]
struct ListBoxPalette {
    row_background: Color,
    row_text: Color,
    row_metadata: Color,
    selected_background: Color,
    hover_background: Color,
    disabled_background: Color,
    disabled_text: Color,
    accent: Color,
}

impl Default for ListBoxPalette {
    fn default() -> Self {
        Self {
            row_background: Color {
                r: 0x1E,
                g: 0x1E,
                b: 0x1C,
            },
            row_text: Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            },
            row_metadata: Color {
                r: 0xB0,
                g: 0xAE,
                b: 0xA5,
            },
            selected_background: Color {
                r: 0x3D,
                g: 0x3D,
                b: 0x3A,
            },
            hover_background: Color {
                r: 0x2A,
                g: 0x2A,
                b: 0x28,
            },
            disabled_background: Color {
                r: 0x1E,
                g: 0x1E,
                b: 0x1C,
            },
            disabled_text: Color {
                r: 0x5E,
                g: 0x5D,
                b: 0x59,
            },
            accent: Color {
                r: 0xC9,
                g: 0x64,
                b: 0x42,
            },
        }
    }
}

fn apply_palette_style(
    palette: &mut ListBoxPalette,
    style_id: StyleId,
    style: &ParsedControlStyle,
) {
    match style_id {
        StyleId::ListBoxRow => {
            if let Some(color) = style.background_color.as_ref() {
                palette.row_background = *color;
            }
            if let Some(color) = style.text_color.as_ref() {
                palette.row_text = *color;
            }
        }
        StyleId::ListBoxSelectedRow => {
            if let Some(color) = style.background_color.as_ref() {
                palette.selected_background = *color;
            }
        }
        StyleId::ListBoxSelectionAccent => {
            if let Some(color) = style.background_color.as_ref() {
                palette.accent = *color;
            }
        }
        StyleId::ListBoxHoverRow => {
            if let Some(color) = style.background_color.as_ref() {
                palette.hover_background = *color;
            }
        }
        StyleId::ListBoxDisabledRow => {
            if let Some(color) = style.background_color.as_ref() {
                palette.disabled_background = *color;
            }
            if let Some(color) = style.text_color.as_ref() {
                palette.disabled_text = *color;
            }
        }
        _ => {}
    }
}

#[derive(Debug)]
struct ListBoxState {
    items: Vec<ListBoxItemDescriptor>,
    selected_index: Option<usize>,
    hover_index: Option<usize>,
    tracking_mouse: bool,
    scroll_row: usize,
    row_height: i32,
    badge_column_width: i32,
    palette: ListBoxPalette,
    title_font: HGDIOBJ,
    meta_font: HGDIOBJ,
}

impl ListBoxState {
    fn new() -> Self {
        let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
        let font = if font.is_invalid() {
            HGDIOBJ::default()
        } else {
            font
        };
        Self {
            items: Vec::new(),
            selected_index: None,
            hover_index: None,
            tracking_mouse: false,
            scroll_row: 0,
            row_height: EXPANDED_ROW_HEIGHT,
            badge_column_width: DEFAULT_BADGE_COLUMN_WIDTH,
            palette: ListBoxPalette::default(),
            title_font: font,
            meta_font: font,
        }
    }
}

fn row_height_for_density(density: ListBoxRowDensity) -> i32 {
    match density {
        ListBoxRowDensity::Expanded => EXPANDED_ROW_HEIGHT,
        ListBoxRowDensity::Compact => COMPACT_ROW_HEIGHT,
    }
}

unsafe fn get_state(hwnd: HWND) -> Option<*mut ListBoxState> {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ListBoxState;
    if ptr.is_null() {
        log::warn!("ListBox state missing for hwnd {hwnd:?}.");
        None
    } else {
        Some(ptr)
    }
}

fn register_list_box_class(h_instance: windows::Win32::Foundation::HINSTANCE) {
    LIST_BOX_CLASS_REGISTERED.get_or_init(|| unsafe {
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(list_box_wnd_proc),
            hInstance: h_instance,
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(std::ptr::null_mut()),
            lpszClassName: LIST_BOX_CLASS_NAME,
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);
    });
}

unsafe extern "system" fn list_box_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    ffi_safety::catch_unwind_ffi(
        "list_box_wnd_proc",
        || match msg {
            WM_NCCREATE => {
                let Some(state_ptr) = (unsafe {
                    DeferredWindowState::<ListBoxState>::adopt_from_create_lparam(lparam)
                }) else {
                    log::error!("ListBox WM_NCCREATE missing initial state for hwnd {hwnd:?}.");
                    return LRESULT(0);
                };
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
                }
                LRESULT(1)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
                if !hdc.is_invalid() {
                    unsafe { paint_list_box(hwnd, hdc) };
                }
                let _ = unsafe { EndPaint(hwnd, &ps) };
                LRESULT(0)
            }
            WM_SIZE => {
                unsafe {
                    update_scroll_info(hwnd);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let x = get_x_lparam(lparam);
                let y = get_y_lparam(lparam);
                unsafe {
                    focus_on_click(hwnd, KEYBOARD_NAVIGATION);
                    if select_row_from_point(hwnd, x, y) {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let x = get_x_lparam(lparam);
                let y = get_y_lparam(lparam);
                unsafe {
                    let Some(state_ptr) = get_state(hwnd) else {
                        return LRESULT(0);
                    };
                    let state = &mut *state_ptr;
                    let new_hover = hit_test_row(state, y);
                    if new_hover != state.hover_index {
                        let previous_hover = state.hover_index;
                        state.hover_index = new_hover;
                        invalidate_row_transition(hwnd, previous_hover, new_hover);
                    }
                    if !state.tracking_mouse {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut tme);
                        state.tracking_mouse = true;
                    }
                    let _ = x;
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                unsafe {
                    let Some(state_ptr) = get_state(hwnd) else {
                        return LRESULT(0);
                    };
                    let state = &mut *state_ptr;
                    let previous_hover = state.hover_index;
                    state.hover_index = None;
                    state.tracking_mouse = false;
                    invalidate_row_transition(hwnd, previous_hover, None);
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                unsafe {
                    let delta = ((wparam.0 >> 16) as i16) as i32;
                    let rows = if delta > 0 { -3 } else { 3 };
                    scroll_by_rows(hwnd, rows);
                }
                LRESULT(0)
            }
            WM_VSCROLL => {
                unsafe {
                    handle_vscroll(hwnd, wparam);
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                unsafe {
                    // Virtual-key codes are WORD-sized in Win32, so narrowing WPARAM here is lossless.
                    let key_code = wparam.0 as u16;
                    if !handle_keydown(hwnd, key_code) {
                        notify_keydown(hwnd, key_code);
                    }
                }
                LRESULT(0)
            }
            WM_GETDLGCODE => dialog_code(KEYBOARD_NAVIGATION)
                .unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }),
            WM_NCDESTROY => {
                let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
                if ptr != 0 {
                    let _ = unsafe { Box::from_raw(ptr as *mut ListBoxState) };
                    unsafe {
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    }
                }
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        },
        || unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    )
}

unsafe fn handle_vscroll(hwnd: HWND, wparam: WPARAM) {
    let code = (wparam.0 & 0xFFFF) as u32;
    let thumb = (wparam.0 >> 16) & 0xFFFF;
    match code {
        x if x == SB_LINEUP.0 as u32 => scroll_by_rows(hwnd, -1),
        x if x == SB_LINEDOWN.0 as u32 => scroll_by_rows(hwnd, 1),
        x if x == SB_PAGEUP.0 as u32 => scroll_by_rows(hwnd, -(visible_rows(hwnd) as i32)),
        x if x == SB_PAGEDOWN.0 as u32 => scroll_by_rows(hwnd, visible_rows(hwnd) as i32),
        x if x == SB_THUMBTRACK.0 as u32 || x == SB_THUMBPOSITION.0 as u32 => {
            set_scroll_row(hwnd, thumb);
        }
        x if x == SB_TOP.0 as u32 => {
            set_scroll_row(hwnd, 0);
        }
        x if x == SB_BOTTOM.0 as u32 => {
            let max = {
                let Some(state_ptr) = get_state(hwnd) else {
                    return;
                };
                let state = &mut *state_ptr;
                state.items.len().saturating_sub(visible_rows(hwnd))
            };
            set_scroll_row(hwnd, max);
        }
        _ => {}
    }
}

fn is_navigation_key(key: u16) -> bool {
    matches!(
        key,
        x if x == VK_UP.0
            || x == VK_DOWN.0
            || x == VK_HOME.0
            || x == VK_END.0
            || x == VK_PRIOR.0
            || x == VK_NEXT.0
    )
}

unsafe fn handle_keydown(hwnd: HWND, key: u16) -> bool {
    let Some(state_ptr) = get_state(hwnd) else {
        return false;
    };
    let state = &mut *state_ptr;
    let len = state.items.len();
    if len == 0 {
        return false;
    }
    if !is_navigation_key(key) {
        return false;
    }
    let visible = visible_rows(hwnd).max(1);
    let Some(next) = next_navigation_index(state.selected_index, len, visible, key) else {
        return true;
    };
    if state.selected_index != Some(next) {
        state.selected_index = Some(next);
        ensure_row_visible(hwnd, next);
        notify_selection_changed(hwnd, state.items[next].id);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    true
}

unsafe fn scroll_by_rows(hwnd: HWND, delta: i32) {
    let Some(state_ptr) = get_state(hwnd) else {
        return;
    };
    let state = &mut *state_ptr;
    let next = (state.scroll_row as i32).saturating_add(delta).max(0) as usize;
    set_scroll_row(hwnd, next);
}

unsafe fn set_scroll_row(hwnd: HWND, row: usize) {
    let Some(state_ptr) = get_state(hwnd) else {
        return;
    };
    let state = &mut *state_ptr;
    let max_row = state.items.len().saturating_sub(visible_rows(hwnd));
    state.scroll_row = row.min(max_row);
    update_scroll_info(hwnd);
    let _ = InvalidateRect(Some(hwnd), None, false);
    notify_scroll_changed(hwnd, u32_from_usize_saturating(state.scroll_row));
}

unsafe fn visible_rows(hwnd: HWND) -> usize {
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let height = client.bottom - client.top;
    let row_height = get_state(hwnd)
        .map(|state_ptr| (&*state_ptr).row_height.max(1))
        .unwrap_or(EXPANDED_ROW_HEIGHT);
    (height / row_height).max(1) as usize
}

unsafe fn update_scroll_info(hwnd: HWND) {
    let Some(state_ptr) = get_state(hwnd) else {
        return;
    };
    let state = &mut *state_ptr;
    let visible = visible_rows(hwnd).max(1);
    let max = i32_from_usize_saturating(state.items.len().saturating_sub(visible));
    let si = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: max,
        nPage: u32_from_usize_saturating(visible),
        nPos: i32_from_usize_saturating(state.scroll_row),
        ..Default::default()
    };
    let _ = SetScrollInfo(hwnd, SB_VERT, &si, true);
}

unsafe fn ensure_row_visible(hwnd: HWND, row: usize) {
    let Some(state_ptr) = get_state(hwnd) else {
        return;
    };
    let state = &mut *state_ptr;
    let visible = visible_rows(hwnd).max(1);
    if row < state.scroll_row {
        set_scroll_row(hwnd, row);
    } else if row >= state.scroll_row + visible {
        set_scroll_row(hwnd, row + 1 - visible);
    }
}

fn hit_test_row(state: &ListBoxState, y: i32) -> Option<usize> {
    let row = (y / state.row_height.max(1)).max(0) as usize + state.scroll_row;
    (row < state.items.len()).then_some(row)
}

/// Compute the next selected index for a navigation key. Index-based only - it
/// deliberately does not consult `ListBoxItemDescriptor::enabled`, so disabled
/// rows remain reachable by keyboard. Returns `None` when there is nothing to move to.
fn next_navigation_index(
    selected: Option<usize>,
    len: usize,
    visible: usize,
    key: u16,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let current = selected.unwrap_or(0);
    let next = if key == VK_UP.0 {
        current.saturating_sub(1)
    } else if key == VK_DOWN.0 {
        (current + 1).min(len.saturating_sub(1))
    } else if key == VK_HOME.0 {
        0
    } else if key == VK_END.0 {
        len.saturating_sub(1)
    } else if key == VK_PRIOR.0 {
        current.saturating_sub(visible)
    } else if key == VK_NEXT.0 {
        (current + visible).min(len.saturating_sub(1))
    } else {
        return None;
    };
    Some(next)
}

fn row_rect(state: &ListBoxState, row_index: usize, width: i32) -> Option<RECT> {
    if row_index < state.scroll_row {
        return None;
    }
    let visible_offset = row_index - state.scroll_row;
    let row_height = state.row_height.max(1);
    let top = i32::try_from(visible_offset)
        .ok()?
        .saturating_mul(row_height);
    Some(RECT {
        left: 0,
        top,
        right: width.max(0),
        bottom: top + row_height,
    })
}

unsafe fn invalidate_row(hwnd: HWND, row_index: usize) {
    let Some(state_ptr) = get_state(hwnd) else {
        return;
    };
    let state = &*state_ptr;
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    if let Some(rect) = row_rect(state, row_index, client.right - client.left) {
        let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
    }
}

unsafe fn invalidate_row_transition(hwnd: HWND, previous: Option<usize>, current: Option<usize>) {
    if previous == current {
        return;
    }
    if let Some(row) = previous {
        invalidate_row(hwnd, row);
    }
    if let Some(row) = current {
        invalidate_row(hwnd, row);
    }
}

unsafe fn select_row_from_point(hwnd: HWND, _x: i32, y: i32) -> bool {
    let Some(state_ptr) = get_state(hwnd) else {
        return false;
    };
    let state = &mut *state_ptr;
    let Some(row) = hit_test_row(state, y) else {
        return false;
    };
    if state.selected_index != Some(row) {
        state.selected_index = Some(row);
        ensure_row_visible(hwnd, row);
        notify_selection_changed(hwnd, state.items[row].id);
        true
    } else {
        false
    }
}

unsafe fn notify_selection_changed(hwnd: HWND, item_id: ListBoxItemId) {
    let root = GetAncestor(hwnd, GET_ANCESTOR_FLAGS(2));
    if root.is_invalid() {
        return;
    }
    let control_id = GetDlgCtrlID(hwnd);
    if control_id == 0 {
        return;
    }
    let _ = SendMessageW(
        root,
        WM_APP_LISTBOX_SELECTION_CHANGED,
        Some(WPARAM(hwnd.0 as usize)),
        Some(LPARAM(item_id.0 as isize)),
    );
}

unsafe fn notify_scroll_changed(hwnd: HWND, position: u32) {
    let root = GetAncestor(hwnd, GET_ANCESTOR_FLAGS(2));
    if root.is_invalid() {
        return;
    }
    let control_id = GetDlgCtrlID(hwnd);
    if control_id == 0 {
        return;
    }
    let _ = SendMessageW(
        root,
        WM_APP_LISTBOX_SCROLLED,
        Some(WPARAM(hwnd.0 as usize)),
        Some(LPARAM(position as isize)),
    );
}

unsafe fn notify_keydown(hwnd: HWND, key_code: u16) {
    let root = GetAncestor(hwnd, GET_ANCESTOR_FLAGS(2));
    if root.is_invalid() {
        return;
    }
    let control_id = GetDlgCtrlID(hwnd);
    if control_id == 0 {
        return;
    }
    let _ = SendMessageW(
        root,
        WM_APP_LISTBOX_KEYDOWN,
        Some(WPARAM(hwnd.0 as usize)),
        Some(LPARAM(key_code as isize)),
    );
}

fn badge_colors(style: StyleId, disabled: bool) -> ColorPair {
    let pair = match style {
        StyleId::BadgePriorityCritical => ColorPair {
            background: Color {
                r: 0x8F,
                g: 0x2D,
                b: 0x2E,
            },
            text: Color {
                r: 0xFF,
                g: 0xF7,
                b: 0xF4,
            },
        },
        StyleId::BadgePriorityHigh => ColorPair {
            background: Color {
                r: 0xA7,
                g: 0x72,
                b: 0x2A,
            },
            text: Color {
                r: 0xFF,
                g: 0xF8,
                b: 0xEA,
            },
        },
        StyleId::BadgePriorityMedium => ColorPair {
            background: Color {
                r: 0x66,
                g: 0x4B,
                b: 0x8D,
            },
            text: Color {
                r: 0xF1,
                g: 0xE9,
                b: 0xFF,
            },
        },
        StyleId::BadgePriorityLow => ColorPair {
            background: Color {
                r: 0x56,
                g: 0x5C,
                b: 0x66,
            },
            text: Color {
                r: 0xF0,
                g: 0xF3,
                b: 0xF5,
            },
        },
        StyleId::BadgeCategory => ColorPair {
            background: Color {
                r: 0x3D,
                g: 0x45,
                b: 0x43,
            },
            text: Color {
                r: 0xD6,
                g: 0xDC,
                b: 0xD7,
            },
        },
        StyleId::BadgeStatusDone => ColorPair {
            background: Color {
                r: 0x2C,
                g: 0x6C,
                b: 0x4A,
            },
            text: Color {
                r: 0xEC,
                g: 0xF8,
                b: 0xEF,
            },
        },
        StyleId::BadgeStatusError => ColorPair {
            background: Color {
                r: 0x8C,
                g: 0x36,
                b: 0x36,
            },
            text: Color {
                r: 0xFF,
                g: 0xF0,
                b: 0xF0,
            },
        },
        StyleId::BadgeStatusActive => ColorPair {
            background: Color {
                r: 0x5A,
                g: 0x4A,
                b: 0x8F,
            },
            text: Color {
                r: 0xF4,
                g: 0xEF,
                b: 0xFF,
            },
        },
        StyleId::BadgeStatusMuted | StyleId::BadgeIndirect => ColorPair {
            background: Color {
                r: 0x54,
                g: 0x58,
                b: 0x5E,
            },
            text: Color {
                r: 0xE0,
                g: 0xE5,
                b: 0xEC,
            },
        },
        _ => ColorPair {
            background: Color {
                r: 0x54,
                g: 0x58,
                b: 0x5E,
            },
            text: Color {
                r: 0xE0,
                g: 0xE5,
                b: 0xEC,
            },
        },
    };
    if disabled {
        ColorPair {
            background: Color {
                r: (pair.background.r as u16 * 70 / 100) as u8,
                g: (pair.background.g as u16 * 70 / 100) as u8,
                b: (pair.background.b as u16 * 70 / 100) as u8,
            },
            text: Color {
                r: (pair.text.r as u16 * 70 / 100) as u8,
                g: (pair.text.g as u16 * 70 / 100) as u8,
                b: (pair.text.b as u16 * 70 / 100) as u8,
            },
        }
    } else {
        pair
    }
}

unsafe fn paint_list_box(hwnd: HWND, hdc: HDC) {
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ListBoxState;
    if state_ptr.is_null() {
        return;
    }
    let state = unsafe { &mut *state_ptr };
    let mut client = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    let width = client.right - client.left;
    let height = client.bottom - client.top;
    if width <= 0 || height <= 0 {
        return;
    }

    let bg_brush = unsafe { CreateSolidBrush(color_to_colorref(&state.palette.row_background)) };
    let _ = unsafe { FillRect(hdc, &client, bg_brush) };
    let _ = unsafe { DeleteObject(bg_brush.into()) };
    unsafe { SetBkMode(hdc, TRANSPARENT) };

    let start = state.scroll_row.min(state.items.len());
    let end = (start + visible_rows(hwnd).max(1)).min(state.items.len());
    let row_height = state.row_height.max(1);
    for (offset, item) in state.items[start..end].iter().enumerate() {
        let row_index = start + offset;
        let top = (offset as i32) * row_height;
        let row_rect = RECT {
            left: 0,
            top,
            right: width,
            bottom: top + row_height,
        };
        let bg = if state.selected_index == Some(row_index) {
            state.palette.selected_background
        } else if state.hover_index == Some(row_index) {
            state.palette.hover_background
        } else if !item.enabled {
            state.palette.disabled_background
        } else {
            state.palette.row_background
        };
        let row_brush = unsafe { CreateSolidBrush(color_to_colorref(&bg)) };
        let _ = unsafe { FillRect(hdc, &row_rect, row_brush) };
        let _ = unsafe { DeleteObject(row_brush.into()) };

        if state.selected_index == Some(row_index) {
            let accent = unsafe { CreateSolidBrush(color_to_colorref(&state.palette.accent)) };
            let accent_rect = RECT {
                left: 0,
                top,
                right: ACCENT_WIDTH,
                bottom: top + row_height,
            };
            let _ = unsafe { FillRect(hdc, &accent_rect, accent) };
            let _ = unsafe { DeleteObject(accent.into()) };
        }

        draw_badges(hdc, state, item, top);

        let has_metadata = !item.metadata.trim().is_empty();
        let title_color = if item.enabled {
            state.palette.row_text
        } else {
            state.palette.disabled_text
        };
        let meta_color = if item.enabled {
            state.palette.row_metadata
        } else {
            state.palette.disabled_text
        };
        let title_rect = RECT {
            left: state.badge_column_width + ROW_PAD_LEFT,
            top: if has_metadata { top + ROW_PAD_TOP } else { top },
            right: width - ROW_PAD_RIGHT,
            bottom: if has_metadata {
                top + ROW_PAD_TOP + 18
            } else {
                top + row_height
            },
        };

        unsafe {
            let _title_font = SelectedObject::select(hdc, state.title_font);
            SetTextColor(hdc, color_to_colorref(&title_color));
            let mut title: Vec<u16> = item.title.encode_utf16().collect();
            let _ = DrawTextW(
                hdc,
                &mut title[..],
                &mut RECT { ..title_rect },
                windows::Win32::Graphics::Gdi::DT_SINGLELINE
                    | windows::Win32::Graphics::Gdi::DT_VCENTER
                    | windows::Win32::Graphics::Gdi::DT_NOPREFIX
                    | windows::Win32::Graphics::Gdi::DT_END_ELLIPSIS,
            );
        }
        if has_metadata {
            let meta_rect = RECT {
                left: state.badge_column_width + ROW_PAD_LEFT,
                top: top + ROW_PAD_TOP + 18,
                right: width - ROW_PAD_RIGHT,
                bottom: top + row_height - 5,
            };
            unsafe {
                let _meta_font = SelectedObject::select(hdc, state.meta_font);
                SetTextColor(hdc, color_to_colorref(&meta_color));
                let mut meta: Vec<u16> = item.metadata.encode_utf16().collect();
                let _ = DrawTextW(
                    hdc,
                    &mut meta[..],
                    &mut RECT { ..meta_rect },
                    windows::Win32::Graphics::Gdi::DT_SINGLELINE
                        | windows::Win32::Graphics::Gdi::DT_VCENTER
                        | windows::Win32::Graphics::Gdi::DT_NOPREFIX
                        | windows::Win32::Graphics::Gdi::DT_END_ELLIPSIS,
                );
            }
        }
    }
}

fn draw_badges(hdc: HDC, state: &ListBoxState, item: &ListBoxItemDescriptor, top: i32) {
    if item.badges.is_empty() {
        return;
    }
    let mut x = ROW_PAD_LEFT;
    let badge_column_right = ROW_PAD_LEFT + state.badge_column_width - BADGE_GAP;
    let y = top + (state.row_height.max(1) - BADGE_HEIGHT) / 2;
    for badge in &item.badges {
        if x >= badge_column_right {
            break;
        }
        let pair = badge_colors(badge.style, !item.enabled);
        let mut text: Vec<u16> = badge.text.encode_utf16().collect();
        let mut size = SIZE::default();
        // Keep meta_font selected for both measurement and drawing.
        let _font = unsafe { SelectedObject::select(hdc, state.meta_font) };
        unsafe {
            let _ = GetTextExtentPoint32W(hdc, &text, &mut size);
        }
        let badge_width = size.cx + BADGE_PAD_X * 2;
        let rect = RECT {
            left: x,
            top: y,
            right: (x + badge_width).min(badge_column_right),
            bottom: y + BADGE_HEIGHT,
        };
        if rect.right <= rect.left {
            break; // _font drops here, restoring previous selection
        }
        unsafe {
            let fill = CreateSolidBrush(color_to_colorref(&pair.background));
            let null_pen = GetStockObject(windows::Win32::Graphics::Gdi::NULL_PEN);
            let _pen = SelectedObject::select(hdc, null_pen);
            let _brush = SelectedObject::select(hdc, fill.into());
            let _ = RoundRect(
                hdc,
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                BADGE_RADIUS,
                BADGE_RADIUS,
            );
            // _brush and _pen drop here, restoring previous pen/brush
            drop(_brush);
            drop(_pen);
            let _ = DeleteObject(fill.into());
            SetTextColor(hdc, color_to_colorref(&pair.text));
            let mut text_rect = badge_text_rect(rect);
            let _ = DrawTextW(
                hdc,
                &mut text[..],
                &mut text_rect,
                windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT(badge_text_flags()),
            );
        }
        // _font drops at end of loop body, restoring previous font selection
        x += badge_width + BADGE_GAP;
    }
}

fn badge_text_rect(rect: RECT) -> RECT {
    let left = (rect.left + BADGE_PAD_X).min(rect.right);
    let right = (rect.right - BADGE_PAD_X).max(left);
    RECT {
        left,
        top: rect.top,
        right,
        bottom: rect.bottom,
    }
}

fn badge_text_flags() -> u32 {
    windows::Win32::Graphics::Gdi::DT_SINGLELINE.0
        | windows::Win32::Graphics::Gdi::DT_LEFT.0
        | windows::Win32::Graphics::Gdi::DT_VCENTER.0
        | windows::Win32::Graphics::Gdi::DT_NOPREFIX.0
        | windows::Win32::Graphics::Gdi::DT_END_ELLIPSIS.0
}

pub(crate) fn handle_create_list_box_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
) -> PlatformResult<()> {
    let parent_hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        if window_data.has_control(control_id) {
            return Err(PlatformError::OperationFailed(format!(
                "ListBox {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        let hwnd_parent = match parent_control_id {
            Some(id) => window_data.get_control_hwnd(id).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "Parent control {} not found in WinID {window_id:?}",
                    id.raw()
                ))
            })?,
            None => window_data.get_hwnd(),
        };
        if hwnd_parent.is_invalid() {
            return Err(PlatformError::InvalidHandle(format!(
                "Parent HWND invalid WinID={window_id:?}"
            )));
        }
        Ok(hwnd_parent)
    })?;

    let h_instance = internal_state.h_instance();
    register_list_box_class(h_instance);
    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            return Err(PlatformError::OperationFailed(format!(
                "ListBox {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        window_data.register_control_kind(control_id, ControlKind::ListBox);
        Ok(())
    })?;

    let create_state = DeferredWindowState::new(ListBoxState::new());
    let create_state_raw = create_state.into_raw();
    let hwnd = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            LIST_BOX_CLASS_NAME,
            &HSTRING::from(""),
            apply_window_style(WS_CHILD | WS_VISIBLE | WS_VSCROLL, KEYBOARD_NAVIGATION),
            0,
            0,
            10,
            10,
            Some(parent_hwnd),
            Some(HMENU(control_id.raw() as usize as *mut std::ffi::c_void)),
            Some(h_instance),
            Some(create_state_raw as *mut _),
        ) {
            Ok(hwnd) => hwnd,
            Err(err) => {
                DeferredWindowState::<ListBoxState>::reclaim(create_state_raw);
                let _ = internal_state.with_window_data_write(window_id, |window_data| {
                    window_data.unregister_control_kind(control_id);
                    Ok(())
                });
                return Err(err.into());
            }
        }
    };
    unsafe {
        DeferredWindowState::<ListBoxState>::reclaim(create_state_raw);
    }

    try_enable_dark_mode(hwnd);
    internal_state.with_window_data_write(window_id, |window_data| {
        window_data.register_control_hwnd(control_id, hwnd);
        Ok(())
    })?;
    Ok(())
}

pub(crate) fn handle_populate_list_box_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    items: Vec<ListBoxItemDescriptor>,
    badge_column_width: u16,
) -> PlatformResult<()> {
    let hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "ListBox control {} not found in window {window_id:?}",
                control_id.raw()
            ))
        })
    })?;
    unsafe {
        let state = get_state(hwnd).ok_or_else(|| {
            PlatformError::OperationFailed(format!(
                "ListBox state missing for control {} in window {window_id:?}",
                control_id.raw()
            ))
        })?;
        let state = &mut *state;
        let selected_id = state
            .selected_index
            .and_then(|index| state.items.get(index))
            .map(|item| item.id);
        state.items = items;
        state.badge_column_width = i32::from(badge_column_width);
        state.selected_index =
            selected_id.and_then(|id| state.items.iter().position(|item| item.id == id));
        if state.items.is_empty() {
            state.scroll_row = 0;
        } else {
            let max = state.items.len().saturating_sub(visible_rows(hwnd));
            state.scroll_row = state.scroll_row.min(max);
        }
        update_scroll_info(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    Ok(())
}

pub(crate) fn handle_set_list_box_row_density_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    density: ListBoxRowDensity,
) -> PlatformResult<()> {
    let hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "ListBox control {} not found in window {window_id:?}",
                control_id.raw()
            ))
        })
    })?;
    unsafe {
        let state = get_state(hwnd).ok_or_else(|| {
            PlatformError::OperationFailed(format!(
                "ListBox state missing for control {} in window {window_id:?}",
                control_id.raw()
            ))
        })?;
        let state = &mut *state;
        state.row_height = row_height_for_density(density);
        if state.items.is_empty() {
            state.scroll_row = 0;
        } else {
            let max = state.items.len().saturating_sub(visible_rows(hwnd));
            state.scroll_row = state.scroll_row.min(max);
        }
        update_scroll_info(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    Ok(())
}

pub(crate) fn handle_set_list_box_selection_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    item_id: ListBoxItemId,
) -> PlatformResult<()> {
    let hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "ListBox control {} not found in window {window_id:?}",
                control_id.raw()
            ))
        })
    })?;
    unsafe {
        let state = get_state(hwnd).ok_or_else(|| {
            PlatformError::OperationFailed(format!(
                "ListBox state missing for control {} in window {window_id:?}",
                control_id.raw()
            ))
        })?;
        let state = &mut *state;
        let Some(index) = state.items.iter().position(|item| item.id == item_id) else {
            return Err(PlatformError::InvalidHandle(format!(
                "ListBox item {:?} not found for control {}",
                item_id,
                control_id.raw()
            )));
        };
        state.selected_index = Some(index);
        ensure_row_visible(hwnd, index);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    Ok(())
}

pub(crate) fn handle_apply_style_command(
    hwnd: HWND,
    style_id: StyleId,
    parsed_style: &ParsedControlStyle,
) {
    unsafe {
        if let Some(state_ptr) = get_state(hwnd) {
            let state = &mut *state_ptr;
            apply_palette_style(&mut state.palette, style_id, parsed_style);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_is_dark() {
        let palette = ListBoxPalette::default();
        assert!(palette.row_background.r < 0x80);
        assert!(palette.row_text.r > palette.row_background.r);
        assert!(palette.accent.r > palette.accent.b);
    }

    #[test]
    fn apply_palette_style_updates_selection_accent() {
        let mut palette = ListBoxPalette::default();
        let style = ParsedControlStyle {
            background_color: Some(Color {
                r: 0xC9,
                g: 0x64,
                b: 0x42,
            }),
            ..Default::default()
        };

        apply_palette_style(&mut palette, StyleId::ListBoxSelectionAccent, &style);

        assert_eq!(palette.accent.r, 0xC9);
        assert_eq!(palette.accent.g, 0x64);
        assert_eq!(palette.accent.b, 0x42);
    }

    #[test]
    fn badge_text_rect_reserves_horizontal_padding() {
        let rect = RECT {
            left: 12,
            top: 4,
            right: 112,
            bottom: 20,
        };

        let text_rect = badge_text_rect(rect);

        assert_eq!(text_rect.left, 18);
        assert_eq!(text_rect.right, 106);
        assert_eq!(text_rect.top, rect.top);
        assert_eq!(text_rect.bottom, rect.bottom);
    }

    #[test]
    fn badge_text_rect_clamps_when_badge_is_too_narrow() {
        let rect = RECT {
            left: 0,
            top: 0,
            right: 8,
            bottom: 16,
        };

        let text_rect = badge_text_rect(rect);

        assert_eq!(text_rect.left, 6);
        assert_eq!(text_rect.right, 6);
    }

    #[test]
    fn badge_text_flags_left_align_with_end_ellipsis() {
        let flags = badge_text_flags();

        assert_eq!(
            flags & windows::Win32::Graphics::Gdi::DT_LEFT.0,
            windows::Win32::Graphics::Gdi::DT_LEFT.0
        );
        assert_eq!(
            flags & windows::Win32::Graphics::Gdi::DT_END_ELLIPSIS.0,
            windows::Win32::Graphics::Gdi::DT_END_ELLIPSIS.0
        );
        assert_eq!(flags & windows::Win32::Graphics::Gdi::DT_CENTER.0, 0);
    }

    #[test]
    fn row_rect_maps_visible_rows_to_client_coordinates() {
        let state = ListBoxState {
            scroll_row: 5,
            ..ListBoxState::new()
        };

        let rect = row_rect(&state, 7, 320).expect("visible row rect");

        assert_eq!(rect.left, 0);
        assert_eq!(rect.top, 2 * EXPANDED_ROW_HEIGHT);
        assert_eq!(rect.right, 320);
        assert_eq!(rect.bottom, 3 * EXPANDED_ROW_HEIGHT);
    }

    #[test]
    fn row_density_maps_to_expected_height() {
        assert_eq!(
            row_height_for_density(ListBoxRowDensity::Compact),
            COMPACT_ROW_HEIGHT
        );
        assert_eq!(
            row_height_for_density(ListBoxRowDensity::Expanded),
            EXPANDED_ROW_HEIGHT
        );
    }

    #[test]
    fn navigation_key_helper_covers_list_navigation_keys() {
        assert!(is_navigation_key(VK_UP.0));
        assert!(is_navigation_key(VK_DOWN.0));
        assert!(is_navigation_key(VK_HOME.0));
        assert!(is_navigation_key(VK_END.0));
        assert!(is_navigation_key(VK_PRIOR.0));
        assert!(is_navigation_key(VK_NEXT.0));
        assert!(!is_navigation_key(b'X' as u16));
    }

    #[test]
    fn next_navigation_index_moves_across_disabled_rows() {
        let next = next_navigation_index(Some(0), 2, 1, VK_DOWN.0);

        assert_eq!(next, Some(1));
    }

    #[test]
    fn next_navigation_index_respects_home_end_and_page_keys() {
        assert_eq!(next_navigation_index(Some(3), 5, 2, VK_HOME.0), Some(0));
        assert_eq!(next_navigation_index(Some(1), 5, 2, VK_END.0), Some(4));
        assert_eq!(next_navigation_index(Some(4), 5, 2, VK_PRIOR.0), Some(2));
        assert_eq!(next_navigation_index(Some(1), 5, 2, VK_NEXT.0), Some(3));
    }

    #[test]
    fn next_navigation_index_returns_none_for_unknown_keys_and_empty_lists() {
        assert_eq!(next_navigation_index(Some(0), 0, 1, VK_DOWN.0), None);
        assert_eq!(next_navigation_index(Some(0), 2, 1, b'X' as u16), None);
    }
}
