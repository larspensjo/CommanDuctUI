/*
 * Custom-WndProc toggle switch control for CommanDuctUI.
 *
 * Renders a sliding pill + knob toggle switch, fully owner-drawn.
 * Sends WM_APP_TOGGLE_SWITCH_CLICKED to the root window when toggled
 * by click (WM_LBUTTONUP) or keyboard (VK_SPACE / VK_RETURN).
 *
 * Per-instance state is stored in GWLP_USERDATA as a heap-allocated
 * `ToggleSwitchState`, matching the pattern used by `tab_bar_handler`
 * and `chart_handler`.
 */

use crate::app::Win32ApiInternalState;
use crate::controls::styling_handler::color_to_colorref;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::styling::Color;
use crate::types::{ControlId, WindowId};
use crate::window_common::{ControlKind, WM_APP_TOGGLE_SWITCH_CLICKED, try_enable_dark_mode};

use std::sync::{Arc, OnceLock};

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CreatePen, CreateSolidBrush, DEFAULT_GUI_FONT, DeleteObject, DrawFocusRect,
        Ellipse, EndPaint, FillRect, GetStockObject, HDC, HGDIOBJ, InvalidateRect, PAINTSTRUCT,
        PS_SOLID, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
    },
    UI::{
        Input::KeyboardAndMouse::{VK_RETURN, VK_SPACE},
        WindowsAndMessaging::{
            CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, GET_ANCESTOR_FLAGS,
            GWLP_USERDATA, GetAncestor, GetClientRect, GetWindowLongPtrW, HMENU, RegisterClassW,
            SendMessageW, SetWindowLongPtrW, WINDOW_EX_STYLE, WM_DESTROY,
            WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONUP, WM_PAINT, WM_SETFOCUS,
            WNDCLASSW, WS_CHILD, WS_TABSTOP, WS_VISIBLE,
        },
    },
};
use windows::core::{HSTRING, PCWSTR, w};

// ── Palette ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct ToggleSwitchPalette {
    pub background: Color,
    pub pill_off: Color,
    pub pill_on: Color,
    pub knob: Color,
    pub text: Color,
}

impl Default for ToggleSwitchPalette {
    fn default() -> Self {
        Self {
            background: Color { r: 0x2B, g: 0x2B, b: 0x2B },
            pill_off:   Color { r: 0x4B, g: 0x4F, b: 0x57 },
            pill_on:    Color { r: 0x00, g: 0x80, b: 0xFF },
            knob:       Color { r: 0xF0, g: 0xF0, b: 0xF0 },
            text:       Color { r: 0xCC, g: 0xCC, b: 0xCC },
        }
    }
}

// ── ToggleSwitchState ─────────────────────────────────────────────────────────

/// Per-instance heap-allocated state stored in GWLP_USERDATA.
struct ToggleSwitchState {
    checked: bool,
    focused: bool,
    label: String,
    palette: ToggleSwitchPalette,
    // No event_tx — events are delivered via WM_APP_TOGGLE_SWITCH_CLICKED
    // to the root window WndProc, consistent with tab_bar and splitter patterns.
}

impl ToggleSwitchState {
    fn new(label: String, checked: bool) -> Self {
        Self {
            checked,
            focused: false,
            label,
            palette: ToggleSwitchPalette::default(),
        }
    }
}

// ── Window class ──────────────────────────────────────────────────────────────

const TOGGLE_SWITCH_CLASS_NAME: PCWSTR = w!("HarvesterToggleSwitchClass");
static TOGGLE_SWITCH_CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

fn register_toggle_switch_class(h_instance: windows::Win32::Foundation::HINSTANCE) {
    TOGGLE_SWITCH_CLASS_REGISTERED.get_or_init(|| unsafe {
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(toggle_switch_wnd_proc),
            hInstance: h_instance,
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(std::ptr::null_mut()),
            lpszClassName: TOGGLE_SWITCH_CLASS_NAME,
            hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
                None,
                windows::Win32::UI::WindowsAndMessaging::IDC_HAND,
            )
            .unwrap_or_default(),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);
    });
}

// ── WndProc ───────────────────────────────────────────────────────────────────

// Pill geometry constants (logical pixels).
const PILL_W: i32 = 28;
const PILL_H: i32 = 16;
const KNOB_D: i32 = 12; // knob diameter
const PILL_MARGIN_LEFT: i32 = 6; // distance from left edge of control to pill left
const PILL_LABEL_GAP: i32 = 8; // gap between pill right edge and label text

unsafe extern "system" fn toggle_switch_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            // Suppress default erase — WM_PAINT fills everything, prevents flicker.
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            if !hdc.is_invalid() {
                unsafe { paint_toggle_switch(hwnd, hdc) };
            }
            let _ = unsafe { EndPaint(hwnd, &ps) };
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            // Toggle on mouse button release — standard Windows control behavior:
            // the user can cancel by moving the cursor away before releasing.
            unsafe {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                if ptr != 0 {
                    let state = ptr as *mut ToggleSwitchState;
                    (*state).checked = !(*state).checked;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    let new_checked = (*state).checked;
                    let root = GetAncestor(hwnd, GET_ANCESTOR_FLAGS(2)); // GA_ROOT
                    if !root.is_invalid() {
                        let _ = SendMessageW(
                            root,
                            WM_APP_TOGGLE_SWITCH_CLICKED,
                            Some(WPARAM(hwnd.0 as usize)),
                            Some(LPARAM(new_checked as isize)),
                        );
                    }
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            if vk == VK_SPACE.0 || vk == VK_RETURN.0 {
                unsafe {
                    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                    if ptr != 0 {
                        let state = ptr as *mut ToggleSwitchState;
                        (*state).checked = !(*state).checked;
                        let _ = InvalidateRect(Some(hwnd), None, false);
                        let new_checked = (*state).checked;
                        let root = GetAncestor(hwnd, GET_ANCESTOR_FLAGS(2)); // GA_ROOT
                        if !root.is_invalid() {
                            let _ = SendMessageW(
                                root,
                                WM_APP_TOGGLE_SWITCH_CLICKED,
                                Some(WPARAM(hwnd.0 as usize)),
                                Some(LPARAM(new_checked as isize)),
                            );
                        }
                    }
                }
                LRESULT(0)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_SETFOCUS => {
            unsafe {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                if ptr != 0 {
                    (*(ptr as *mut ToggleSwitchState)).focused = true;
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            unsafe {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                if ptr != 0 {
                    (*(ptr as *mut ToggleSwitchState)).focused = false;
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
            if ptr != 0 {
                let _ = unsafe { Box::from_raw(ptr as *mut ToggleSwitchState) };
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

// ── Paint ─────────────────────────────────────────────────────────────────────

unsafe fn paint_toggle_switch(hwnd: HWND, hdc: HDC) {
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ToggleSwitchState;
    if state_ptr.is_null() {
        return;
    }
    let state: &ToggleSwitchState = unsafe { &*state_ptr };

    let mut client = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    let h = client.bottom - client.top;
    let w = client.right - client.left;
    if h <= 0 || w <= 0 {
        return;
    }

    // Fill background.
    let bg_cr = color_to_colorref(&state.palette.background);
    let bg_brush = unsafe { CreateSolidBrush(bg_cr) };
    let _ = unsafe { FillRect(hdc, &client, bg_brush) };
    let _ = unsafe { DeleteObject(bg_brush.into()) };

    // Compute pill position (centered vertically).
    let pill_top = (h - PILL_H) / 2;
    let pill_bottom = pill_top + PILL_H;
    let pill_left = PILL_MARGIN_LEFT;
    let pill_right = pill_left + PILL_W;

    // Draw pill using RoundRect (corner radius = PILL_H for fully rounded ends).
    let pill_color = if state.checked {
        color_to_colorref(&state.palette.pill_on)
    } else {
        color_to_colorref(&state.palette.pill_off)
    };
    let pill_brush = unsafe { CreateSolidBrush(pill_color) };
    let null_pen: HGDIOBJ = unsafe { GetStockObject(windows::Win32::Graphics::Gdi::NULL_PEN) };
    let old_pen = unsafe { SelectObject(hdc, null_pen) };
    let old_brush = unsafe { SelectObject(hdc, pill_brush.into()) };
    let corner = PILL_H; // diameter = height → fully rounded ends
    let _ = unsafe {
        RoundRect(hdc, pill_left, pill_top, pill_right, pill_bottom, corner, corner)
    };
    unsafe { SelectObject(hdc, old_brush) };
    unsafe { SelectObject(hdc, old_pen) };
    let _ = unsafe { DeleteObject(pill_brush.into()) };

    // Draw knob (circle, centered vertically inside pill).
    let knob_margin = (PILL_H - KNOB_D) / 2;
    let (knob_left, knob_right) = if state.checked {
        (
            pill_right - knob_margin - KNOB_D,
            pill_right - knob_margin,
        )
    } else {
        (
            pill_left + knob_margin,
            pill_left + knob_margin + KNOB_D,
        )
    };
    let knob_top = pill_top + knob_margin;
    let knob_bottom = knob_top + KNOB_D;

    let knob_cr = color_to_colorref(&state.palette.knob);
    let knob_brush = unsafe { CreateSolidBrush(knob_cr) };
    let knob_pen = unsafe { CreatePen(PS_SOLID, 0, knob_cr) };
    let old_pen = unsafe { SelectObject(hdc, knob_pen.into()) };
    let old_brush = unsafe { SelectObject(hdc, knob_brush.into()) };
    let _ = unsafe { Ellipse(hdc, knob_left, knob_top, knob_right, knob_bottom) };
    unsafe { SelectObject(hdc, old_brush) };
    unsafe { SelectObject(hdc, old_pen) };
    let _ = unsafe { DeleteObject(knob_brush.into()) };
    let _ = unsafe { DeleteObject(knob_pen.into()) };

    // Draw label text to the right of the pill.
    let text_x = pill_right + PILL_LABEL_GAP;
    let label_wide: Vec<u16> = state.label.encode_utf16().collect();
    if !label_wide.is_empty() {
        unsafe { SetBkMode(hdc, TRANSPARENT) };
        let text_cr = color_to_colorref(&state.palette.text);
        let _ = unsafe { SetTextColor(hdc, text_cr) };

        let stock_font: HGDIOBJ = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
        let old_font = unsafe { SelectObject(hdc, stock_font) };

        let mut sz = windows::Win32::Foundation::SIZE::default();
        let _ = unsafe {
            windows::Win32::Graphics::Gdi::GetTextExtentPoint32W(hdc, &label_wide, &mut sz)
        };
        let text_y = (h - sz.cy) / 2;
        let _ = unsafe { TextOutW(hdc, text_x, text_y, &label_wide) };
        unsafe { SelectObject(hdc, old_font) };
    }

    // Draw focus rect around pill when focused.
    if state.focused {
        let focus_rect = RECT {
            left: pill_left - 2,
            top: pill_top - 2,
            right: pill_right + 2,
            bottom: pill_bottom + 2,
        };
        let _ = unsafe { DrawFocusRect(hdc, &focus_rect) };
    }
}

// ── Command handlers ──────────────────────────────────────────────────────────

/// Creates a ToggleSwitch control as a child of `parent_control_id` (or main window if None).
/// Follows the 4-phase read-kind-create-hwnd-write pattern from `tab_bar_handler`.
pub(crate) fn handle_create_toggle_switch_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
    label: String,
    checked: bool,
) -> PlatformResult<()> {
    log::debug!(
        "[ToggleSwitch] handle_create_toggle_switch_command WinID={window_id:?} ControlID={} ParentID={:?}",
        control_id.raw(),
        parent_control_id.map(|id| id.raw()),
    );

    // Phase 1: Read-lock — duplicate check + get parent HWND.
    let parent_hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        if window_data.has_control(control_id) {
            log::warn!(
                "[ToggleSwitch] ToggleSwitch {} already exists for window {window_id:?}.",
                control_id.raw()
            );
            return Err(PlatformError::OperationFailed(format!(
                "ToggleSwitch {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        let hwnd_parent = match parent_control_id {
            Some(id) => window_data.get_control_hwnd(id).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "[ToggleSwitch] Parent control {} not found in WinID {window_id:?}",
                    id.raw()
                ))
            })?,
            None => window_data.get_hwnd(),
        };
        if hwnd_parent.is_invalid() {
            return Err(PlatformError::InvalidHandle(format!(
                "[ToggleSwitch] Parent HWND invalid WinID={window_id:?}"
            )));
        }
        Ok(hwnd_parent)
    })?;

    let h_instance = internal_state.h_instance();
    register_toggle_switch_class(h_instance);

    // Phase 2: Write-lock — register the control kind.
    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            return Err(PlatformError::OperationFailed(format!(
                "[ToggleSwitch] Race: ToggleSwitch {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        window_data.register_control_kind(control_id, ControlKind::ToggleSwitch);
        Ok(())
    })?;

    // Phase 3: Create native HWND outside any lock.
    let hwnd_toggle = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            TOGGLE_SWITCH_CLASS_NAME,
            &HSTRING::from(""),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            0,
            0,
            10,
            10,
            Some(parent_hwnd),
            Some(HMENU(control_id.raw() as *mut _)),
            Some(h_instance),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(err) => {
                let _ = internal_state.with_window_data_write(window_id, |window_data| {
                    window_data.unregister_control_kind(control_id);
                    Ok(())
                });
                return Err(err.into());
            }
        }
    };

    // Initialise GWLP_USERDATA with per-control state.
    let state = Box::new(ToggleSwitchState::new(label, checked));
    unsafe {
        SetWindowLongPtrW(hwnd_toggle, GWLP_USERDATA, Box::into_raw(state) as isize);
    }

    // Enable dark mode title-bar treatment (toggle owns its own painting via WM_PAINT).
    try_enable_dark_mode(hwnd_toggle);

    // Phase 4: Write-lock — store the HWND.
    internal_state.with_window_data_write(window_id, |window_data| {
        window_data.register_control_hwnd(control_id, hwnd_toggle);
        Ok(())
    })?;

    log::debug!(
        "[ToggleSwitch] Created toggle switch {} hwnd={hwnd_toggle:?}",
        control_id.raw()
    );
    Ok(())
}

/// Programmatically sets the checked state and repaints.
pub(crate) fn handle_set_toggle_switch_state_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    checked: bool,
) -> PlatformResult<()> {
    let hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "[ToggleSwitch] SetToggleSwitchState: control {} not found in window {window_id:?}",
                control_id.raw()
            ))
        })
    })?;
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if ptr != 0 {
            let state = ptr as *mut ToggleSwitchState;
            (*state).checked = checked;
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
    Ok(())
}

/// Pushes resolved palette colors into the control so WM_PAINT can render without
/// accessing `Win32ApiInternalState`.
pub(crate) fn handle_set_toggle_switch_style_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    background: Color,
    pill_off: Color,
    pill_on: Color,
    knob: Color,
    text: Color,
) -> PlatformResult<()> {
    let hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "[ToggleSwitch] SetToggleSwitchStyle: control {} not found in window {window_id:?}",
                control_id.raw()
            ))
        })
    })?;
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if ptr != 0 {
            let state = ptr as *mut ToggleSwitchState;
            (*state).palette = ToggleSwitchPalette {
                background,
                pill_off,
                pill_on,
                knob,
                text,
            };
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
    Ok(())
}
