/*
 * Custom-WndProc tab bar control for CommanDuctUI.
 *
 * Renders a row of text tabs with a colored bottom accent line on the active tab
 * and a subtle hover highlight.  Sends WM_APP_TAB_SELECTED to the parent window
 * when the user clicks a tab.
 *
 * Per-instance state is stored in GWLP_USERDATA as a heap-allocated
 * `TabBarState`, matching the pattern used by `chart_handler` and `splitter_handler`.
 */

use crate::app::Win32ApiInternalState;
use crate::controls::styling_handler::color_to_colorref;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::styling::Color;
use crate::styling_primitives::FontDescription;
use crate::types::{ControlId, WindowId};
use crate::window_common::{ControlKind, WM_APP_TAB_SELECTED};

use std::sync::{Arc, OnceLock};

use windows::core::{HSTRING, PCWSTR, w};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
        DEFAULT_GUI_FONT, DEFAULT_QUALITY, DeleteObject, EndPaint, FF_DONTCARE, FillRect,
        FW_BOLD, FW_NORMAL, GetDC, GetDeviceCaps, GetStockObject, GetTextExtentPoint32W, HFONT,
        HDC, HGDIOBJ, InvalidateRect, LOGPIXELSY, OUT_DEFAULT_PRECIS, PAINTSTRUCT, ReleaseDC,
        SelectObject, SetBkMode, SetTextColor, TextOutW, TRANSPARENT,
    },
    System::WindowsProgramming::MulDiv,
    UI::{
        Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent},
        WindowsAndMessaging::{
            CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, GET_ANCESTOR_FLAGS,
            GWLP_USERDATA, GetAncestor, GetClientRect, GetWindowLongPtrW, HMENU, RegisterClassW,
            SendMessageW, SetWindowLongPtrW, WINDOW_EX_STYLE, WM_DESTROY, WM_ERASEBKGND,
            WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_PAINT, WM_SIZE, WNDCLASSW, WS_CHILD, WS_VISIBLE,
        },
    },
};

// WM_MOUSELEAVE is not exported by windows-rs; define the constant directly.
const WM_MOUSELEAVE: u32 = 0x02A3;

// ── Default palette ───────────────────────────────────────────────────────────

fn default_background() -> Color {
    Color { r: 0x2E, g: 0x32, b: 0x39 }
}
fn default_text() -> Color {
    Color { r: 0xE0, g: 0xE5, b: 0xEC }
}
fn default_accent() -> Color {
    Color { r: 0x00, g: 0x80, b: 0xFF }
}

// ── TabBarPalette ─────────────────────────────────────────────────────────────

/// Style-resolved colors stored inside `TabBarState`.
///
/// `text_inactive` and `hover_fill` are derived from the primary colors so
/// all clients derive those the same way.
#[derive(Debug, Clone)]
pub(crate) struct TabBarPalette {
    pub background: Color,
    pub text_active: Color,
    pub text_inactive: Color, // ~40% text + 60% background blend
    pub hover_fill: Color,    // background + ~6% white overlay
    pub accent: Color,
}

impl TabBarPalette {
    /// Construct a palette and derive `text_inactive` and `hover_fill` from
    /// the primary colors.  The derivation is deterministic so it is
    /// independently testable.
    pub(crate) fn new(background: Color, text: Color, accent: Color) -> Self {
        // text_inactive = 40% text color + 60% background color
        let blend = |a: u8, b: u8| -> u8 {
            let a16 = a as u16;
            let b16 = b as u16;
            ((a16 * 40 + b16 * 60) / 100) as u8
        };
        let text_inactive = Color {
            r: blend(text.r, background.r),
            g: blend(text.g, background.g),
            b: blend(text.b, background.b),
        };

        // hover_fill = background + 6% white overlay
        let overlay = |base: u8| -> u8 {
            let base16 = base as u16;
            let extra = (255u16 * 6) / 100; // ~15
            (base16 + extra).min(255) as u8
        };
        let hover_fill = Color {
            r: overlay(background.r),
            g: overlay(background.g),
            b: overlay(background.b),
        };

        Self { background, text_active: text, text_inactive, hover_fill, accent }
    }
}

impl Default for TabBarPalette {
    fn default() -> Self {
        Self::new(default_background(), default_text(), default_accent())
    }
}

// ── TabBarState ───────────────────────────────────────────────────────────────

/// Per-instance heap-allocated state stored in GWLP_USERDATA.
struct TabBarState {
    items: Vec<String>,
    selected_index: usize,
    hover_index: Option<usize>,
    tracking_mouse: bool,
    /// Computed during WM_PAINT, read during hit-testing.
    item_rects: Vec<RECT>,
    palette: TabBarPalette,
    /// Optional style-driven font; if None the control uses DEFAULT_GUI_FONT.
    font: Option<HFONT>,
}

impl TabBarState {
    fn new(items: Vec<String>) -> Self {
        Self {
            items,
            selected_index: 0,
            hover_index: None,
            tracking_mouse: false,
            item_rects: Vec::new(),
            palette: TabBarPalette::default(),
            font: None,
        }
    }
}

impl Drop for TabBarState {
    fn drop(&mut self) {
        if let Some(hfont) = self.font.take().filter(|f| !f.is_invalid()) {
                unsafe {
                    let _ = DeleteObject(hfont.into());
                }
        }
    }
}

/// Gets or lazily allocates state from GWLP_USERDATA (like chart_handler).
unsafe fn get_or_init_state(hwnd: HWND) -> *mut TabBarState {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if ptr == 0 {
            let data = Box::new(TabBarState::new(Vec::new()));
            let raw = Box::into_raw(data);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
            raw
        } else {
            ptr as *mut TabBarState
        }
    }
}

// ── Window class ──────────────────────────────────────────────────────────────

const TAB_BAR_CLASS_NAME: PCWSTR = w!("CommanDuctUITabBar");
static TAB_BAR_CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

fn register_tab_bar_class(h_instance: windows::Win32::Foundation::HINSTANCE) {
    TAB_BAR_CLASS_REGISTERED.get_or_init(|| unsafe {
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(tab_bar_wnd_proc),
            hInstance: h_instance,
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(std::ptr::null_mut()),
            lpszClassName: TAB_BAR_CLASS_NAME,
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);
    });
}

// ── WndProc ───────────────────────────────────────────────────────────────────

unsafe extern "system" fn tab_bar_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            // The WM_PAINT handler fills the entire client area, so suppress
            // the default erase to avoid flicker.
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            if !hdc.is_invalid() {
                unsafe { paint_tab_bar(hwnd, hdc) };
            }
            let _ = unsafe { EndPaint(hwnd, &ps) };
            LRESULT(0)
        }
        WM_SIZE => {
            let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            unsafe {
                let state = get_or_init_state(hwnd);
                let hit = hit_test(&(*state).item_rects, x, y);
                if let Some(idx) = hit.filter(|&i| i != (*state).selected_index) {
                        (*state).selected_index = idx;
                        let _ = InvalidateRect(Some(hwnd), None, false);
                        // Notify root window: WPARAM = our HWND, LPARAM = selected index.
                        // Use GetAncestor(GA_ROOT) so the message reaches the main window's
                        // WndProc even when the tab bar is a grandchild (panel nesting).
                        let root = GetAncestor(hwnd, GET_ANCESTOR_FLAGS(2)); // GA_ROOT
                        if !root.is_invalid() {
                            let _ = SendMessageW(
                                root,
                                WM_APP_TAB_SELECTED,
                                Some(WPARAM(hwnd.0 as usize)),
                                Some(LPARAM(idx as isize)),
                            );
                        }
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            unsafe {
                let state = get_or_init_state(hwnd);
                let new_hover = hit_test(&(*state).item_rects, x, y);
                if new_hover != (*state).hover_index {
                    (*state).hover_index = new_hover;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                if !(*state).tracking_mouse {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    (*state).tracking_mouse = true;
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            unsafe {
                let state = get_or_init_state(hwnd);
                if (*state).hover_index.is_some() {
                    (*state).hover_index = None;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                (*state).tracking_mouse = false;
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
            if ptr != 0 {
                let _ = unsafe { Box::from_raw(ptr as *mut TabBarState) };
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

// ── Hit-test helper ───────────────────────────────────────────────────────────

fn hit_test(rects: &[RECT], x: i32, y: i32) -> Option<usize> {
    for (i, r) in rects.iter().enumerate() {
        if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
            return Some(i);
        }
    }
    None
}

// ── Paint ─────────────────────────────────────────────────────────────────────

/// Paint the tab bar.  Called from WM_PAINT with the HDC already obtained via
/// `BeginPaint`.  All colors come from `TabBarState.palette`.
unsafe fn paint_tab_bar(hwnd: HWND, hdc: HDC) {
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut TabBarState;
    // Return early if state has not been initialised yet (window is being created).
    if state_ptr.is_null() {
        return;
    }
    let state: &mut TabBarState = unsafe { &mut *state_ptr };

    let mut client = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    let w = client.right - client.left;
    let h = client.bottom - client.top;
    if w <= 0 || h <= 0 {
        return;
    }

    // Select font (saves the old one for restoration).
    let stock_font: HGDIOBJ = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    let font_hgdiobj: HGDIOBJ = if let Some(hf) = state.font {
        if hf.is_invalid() { stock_font } else { hf.into() }
    } else {
        stock_font
    };
    let old_font = unsafe { SelectObject(hdc, font_hgdiobj) };

    // Fill background.
    let bg_cr = color_to_colorref(&state.palette.background);
    let bg_brush = unsafe { CreateSolidBrush(bg_cr) };
    let _ = unsafe { FillRect(hdc, &client, bg_brush) };
    let _ = unsafe { DeleteObject(bg_brush.into()) };

    // Compute tab rects using text extents.
    let h_pad = 16i32; // pixels of horizontal padding on each side of the label
    let mut x_cursor = 0i32;
    let mut new_rects: Vec<RECT> = Vec::with_capacity(state.items.len());
    let mut tab_widths: Vec<i32> = Vec::with_capacity(state.items.len());

    for label in &state.items {
        let wide: Vec<u16> = label.encode_utf16().collect();
        let mut sz = SIZE::default();
        let _ = unsafe { GetTextExtentPoint32W(hdc, &wide, &mut sz) };
        tab_widths.push(sz.cx + h_pad * 2);
    }
    for &tw in &tab_widths {
        new_rects.push(RECT { left: x_cursor, top: 0, right: x_cursor + tw, bottom: h });
        x_cursor += tw;
    }
    state.item_rects = new_rects.clone();

    // Draw tabs.
    unsafe { SetBkMode(hdc, TRANSPARENT) };
    let accent_h = 3i32;

    for (i, (tab_rect, label)) in new_rects.iter().zip(state.items.iter()).enumerate() {
        // Hover highlight for non-active hovered tab.
        if state.hover_index == Some(i) && i != state.selected_index {
            let hover_cr = color_to_colorref(&state.palette.hover_fill);
            let hover_brush = unsafe { CreateSolidBrush(hover_cr) };
            let _ = unsafe { FillRect(hdc, tab_rect, hover_brush) };
            let _ = unsafe { DeleteObject(hover_brush.into()) };
        }

        // Text color: bright for active, dimmed for inactive.
        let txt_cr = if i == state.selected_index {
            color_to_colorref(&state.palette.text_active)
        } else {
            color_to_colorref(&state.palette.text_inactive)
        };
        let _ = unsafe { SetTextColor(hdc, txt_cr) };

        // Center text horizontally and vertically in the tab rect.
        let wide: Vec<u16> = label.encode_utf16().collect();
        let mut sz = SIZE::default();
        let _ = unsafe { GetTextExtentPoint32W(hdc, &wide, &mut sz) };
        let text_x = tab_rect.left + (tab_rect.right - tab_rect.left - sz.cx) / 2;
        let text_y = (h - sz.cy) / 2;
        let _ = unsafe { TextOutW(hdc, text_x, text_y, &wide) };
    }

    // Accent line under the active tab.
    if let Some(active_rect) = new_rects.get(state.selected_index) {
        let accent_rect = RECT {
            left: active_rect.left,
            top: h - accent_h,
            right: active_rect.right,
            bottom: h,
        };
        let accent_cr = color_to_colorref(&state.palette.accent);
        let accent_brush = unsafe { CreateSolidBrush(accent_cr) };
        let _ = unsafe { FillRect(hdc, &accent_rect, accent_brush) };
        let _ = unsafe { DeleteObject(accent_brush.into()) };
    }

    // Restore font.
    unsafe { SelectObject(hdc, old_font) };
}

// ── Font creation helper ──────────────────────────────────────────────────────

/// Creates an HFONT from a `FontDescription`.  Returns `Ok(None)` if `font_desc`
/// is `None`.  Follows the same pattern as `Win32ApiInternalState::define_style`.
fn create_hfont(font_desc: &FontDescription) -> PlatformResult<HFONT> {

    let hdc_screen = unsafe { GetDC(None) };
    if hdc_screen.is_invalid() {
        return Err(PlatformError::OperationFailed(
            "TabBar: could not acquire screen DC for font creation".into(),
        ));
    }
    let logical_height = if let Some(pt) = font_desc.size {
        -unsafe { MulDiv(pt, GetDeviceCaps(Some(hdc_screen), LOGPIXELSY), 72) }
    } else {
        0
    };
    unsafe { ReleaseDC(None, hdc_screen) };

    use crate::styling_primitives::FontWeight;
    let weight = match font_desc.weight {
        Some(FontWeight::Bold) => FW_BOLD.0 as i32,
        _ => FW_NORMAL.0 as i32,
    };
    let name = font_desc.name.as_deref().unwrap_or("MS Shell Dlg 2");
    let name_str = HSTRING::from(name);
    let hfont = unsafe {
        CreateFontW(
            logical_height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            DEFAULT_QUALITY,
            FF_DONTCARE.0 as u32,
            &name_str,
        )
    };
    if hfont.is_invalid() {
        return Err(PlatformError::OperationFailed(
            "TabBar: CreateFontW failed".into(),
        ));
    }
    Ok(hfont)
}

// ── Command handlers ──────────────────────────────────────────────────────────

/// Creates a TabBar control as a child of `parent_control_id` (or the main window if None).
/// Follows the 4-phase read-kind-create-hwnd-write pattern from `chart_handler`.
pub(crate) fn handle_create_tab_bar_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    parent_control_id: Option<ControlId>,
    items: Vec<String>,
) -> PlatformResult<()> {
    log::debug!(
        "[TabBar] handle_create_tab_bar_command WinID={window_id:?} ControlID={} ParentID={:?}",
        control_id.raw(),
        parent_control_id.map(|id| id.raw()),
    );

    // Phase 1: Read-lock — duplicate check + get parent HWND.
    let parent_hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        if window_data.has_control(control_id) {
            log::warn!(
                "[TabBar] TabBar {} already exists for window {window_id:?}.",
                control_id.raw()
            );
            return Err(PlatformError::OperationFailed(format!(
                "TabBar {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        let hwnd_parent = match parent_control_id {
            Some(id) => window_data.get_control_hwnd(id).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "[TabBar] Parent control {} not found in WinID {window_id:?}",
                    id.raw()
                ))
            })?,
            None => window_data.get_hwnd(),
        };
        if hwnd_parent.is_invalid() {
            return Err(PlatformError::InvalidHandle(format!(
                "[TabBar] Parent HWND invalid WinID={window_id:?}"
            )));
        }
        Ok(hwnd_parent)
    })?;

    let h_instance = internal_state.h_instance();
    register_tab_bar_class(h_instance);

    // Phase 2: Write-lock — register the control kind.
    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            return Err(PlatformError::OperationFailed(format!(
                "[TabBar] Race: TabBar {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        window_data.register_control_kind(control_id, ControlKind::TabBar);
        Ok(())
    })?;

    // Phase 3: Create native HWND outside any lock.
    let hwnd_tab_bar = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            TAB_BAR_CLASS_NAME,
            &HSTRING::from(""),
            WS_CHILD | WS_VISIBLE,
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

    // Initialise GWLP_USERDATA with items.
    let state = Box::new(TabBarState::new(items));
    unsafe {
        SetWindowLongPtrW(hwnd_tab_bar, GWLP_USERDATA, Box::into_raw(state) as isize);
    }

    // Phase 4: Write-lock — store the HWND.
    internal_state.with_window_data_write(window_id, |window_data| {
        window_data.register_control_hwnd(control_id, hwnd_tab_bar);
        Ok(())
    })?;

    log::debug!(
        "[TabBar] Created tab bar {} hwnd={hwnd_tab_bar:?}",
        control_id.raw()
    );
    Ok(())
}

/// Replaces all tab labels and triggers a repaint.
pub(crate) fn handle_set_tab_bar_items(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    items: Vec<String>,
) -> PlatformResult<()> {
    let hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "[TabBar] SetTabBarItems: control {} not found in window {window_id:?}",
                control_id.raw()
            ))
        })
    })?;
    unsafe {
        let state = get_or_init_state(hwnd);
        (*state).items = items;
        (*state).selected_index = 0;
        (*state).item_rects.clear();
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    Ok(())
}

/// Drives the active tab selection from the reducer (no user event emitted).
pub(crate) fn handle_set_tab_bar_selection(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    selected_index: usize,
) -> PlatformResult<()> {
    let hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "[TabBar] SetTabBarSelection: control {} not found in window {window_id:?}",
                control_id.raw()
            ))
        })
    })?;
    unsafe {
        let state = get_or_init_state(hwnd);
        let clamped = selected_index.min((*state).items.len().saturating_sub(1));
        if (*state).selected_index != clamped {
            (*state).selected_index = clamped;
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
    Ok(())
}

/// Pushes resolved style data into the control so WM_PAINT can paint
/// without accessing `Win32ApiInternalState`.
pub(crate) fn handle_set_tab_bar_style(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    background_color: Color,
    text_color: Color,
    accent_color: Color,
    font_desc: Option<FontDescription>,
) -> PlatformResult<()> {
    let hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "[TabBar] SetTabBarStyle: control {} not found in window {window_id:?}",
                control_id.raw()
            ))
        })
    })?;

    let new_font: Option<HFONT> = match &font_desc {
        Some(fd) => match create_hfont(fd) {
            Ok(hf) => Some(hf),
            Err(e) => {
                log::warn!("[TabBar] SetTabBarStyle: font creation failed: {e:?}; using default");
                None
            }
        },
        None => None,
    };

    unsafe {
        let state = get_or_init_state(hwnd);
        // Drop old font if any.
        if let Some(old_font) = (*state).font.take().filter(|f| !f.is_invalid()) {
                let _ = DeleteObject(old_font.into());
        }
        (*state).palette = TabBarPalette::new(background_color, text_color, accent_color);
        (*state).font = new_font;
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    Ok(())
}

// ── Unit tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_bar_palette_derives_text_inactive_and_hover_fill() {
        let bg = Color { r: 0x2E, g: 0x32, b: 0x39 };
        let text = Color { r: 0xE0, g: 0xE5, b: 0xEC };
        let accent = Color { r: 0x00, g: 0x80, b: 0xFF };
        let palette = TabBarPalette::new(bg.clone(), text.clone(), accent.clone());

        // text_inactive = 40% text + 60% background
        let expected_inactive_r = ((0xE0u16 * 40 + 0x2Eu16 * 60) / 100) as u8;
        let expected_inactive_g = ((0xE5u16 * 40 + 0x32u16 * 60) / 100) as u8;
        let expected_inactive_b = ((0xECu16 * 40 + 0x39u16 * 60) / 100) as u8;
        assert_eq!(palette.text_inactive.r, expected_inactive_r);
        assert_eq!(palette.text_inactive.g, expected_inactive_g);
        assert_eq!(palette.text_inactive.b, expected_inactive_b);

        // hover_fill = background + 6% white (≈15 per channel)
        let extra = (255u16 * 6) / 100;
        assert_eq!(palette.hover_fill.r, (bg.r as u16 + extra).min(255) as u8);
        assert_eq!(palette.hover_fill.g, (bg.g as u16 + extra).min(255) as u8);
        assert_eq!(palette.hover_fill.b, (bg.b as u16 + extra).min(255) as u8);

        // Primary colors are preserved.
        assert_eq!(palette.background.r, bg.r);
        assert_eq!(palette.text_active.r, text.r);
        assert_eq!(palette.accent.r, accent.r);
    }

    #[test]
    fn tab_bar_palette_default_uses_dark_theme_colors() {
        let palette = TabBarPalette::default();
        assert_eq!(palette.background.r, 0x2E);
        assert_eq!(palette.text_active.r, 0xE0);
        assert_eq!(palette.accent.b, 0xFF);
    }

    #[test]
    fn hit_test_returns_correct_index() {
        let rects = vec![
            RECT { left: 0, top: 0, right: 60, bottom: 28 },
            RECT { left: 60, top: 0, right: 130, bottom: 28 },
            RECT { left: 130, top: 0, right: 190, bottom: 28 },
        ];
        assert_eq!(hit_test(&rects, 10, 5), Some(0));
        assert_eq!(hit_test(&rects, 80, 14), Some(1));
        assert_eq!(hit_test(&rects, 160, 20), Some(2));
        assert_eq!(hit_test(&rects, 300, 5), None);
        assert_eq!(hit_test(&rects, 10, 30), None); // below rect
    }

    #[test]
    fn tab_bar_state_defaults_to_first_tab() {
        let state = TabBarState::new(vec!["A".to_string(), "B".to_string()]);
        assert_eq!(state.selected_index, 0);
        assert!(state.hover_index.is_none());
        assert!(!state.tracking_mouse);
        assert!(state.item_rects.is_empty());
    }
}
