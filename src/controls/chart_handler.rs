/*
 * Owner-drawn GDI line chart control for the Trends tab.
 *
 * Uses a custom registered window class ("HarvesterChartControl") with its own
 * WndProc so it can respond to WM_PAINT, WM_ERASEBKGND, and WM_SIZE independently
 * of the standard Win32 control classes.
 *
 * MVP: the paint function draws a hardcoded 13-point trend line so chart
 * infrastructure can be verified before real data wiring.
 *
 * Dark theme palette (Win32 COLORREF = 0x00BBGGRR):
 *   Background  #1E2228  → 0x0028_221E
 *   Gridlines   #3A3F47  → 0x0047_3F3A
 *   Trend line  #4EC9B0  → 0x00B0_C9_4E
 */

use crate::app::Win32ApiInternalState;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::types::{ControlId, WindowId};
use crate::window_common::ControlKind;

use std::sync::{Arc, OnceLock};
use windows::Win32::{
    Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CreatePen, CreateSolidBrush, DeleteObject, EndPaint, FillRect,
        InvalidateRect, PAINTSTRUCT, PS_DOT, PS_SOLID, Polyline, SelectObject,
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, GetClientRect, HMENU, RegisterClassW, WINDOW_EX_STYLE,
        WM_ERASEBKGND, WM_PAINT, WM_SIZE, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN,
        WS_VISIBLE,
    },
};
use windows::core::{HSTRING, PCWSTR, w};

// ── Dark theme colors ─────────────────────────────────────────────────────────

const COLOR_BG: COLORREF = COLORREF(0x0028_221E); // #1E2228
const COLOR_GRID: COLORREF = COLORREF(0x0047_3F3A); // #3A3F47
const COLOR_LINE: COLORREF = COLORREF(0x00B0_C9_4E); // #4EC9B0

// ── Window class ─────────────────────────────────────────────────────────────

const CHART_CLASS_NAME: PCWSTR = w!("HarvesterChartControl");

static CHART_CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

fn register_chart_class(h_instance: HINSTANCE) {
    CHART_CLASS_REGISTERED.get_or_init(|| unsafe {
        let wc = WNDCLASSW {
            style: windows::Win32::UI::WindowsAndMessaging::CS_HREDRAW
                | windows::Win32::UI::WindowsAndMessaging::CS_VREDRAW,
            lpfnWndProc: Some(chart_wnd_proc),
            hInstance: h_instance,
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(std::ptr::null_mut()),
            lpszClassName: CHART_CLASS_NAME,
            ..Default::default()
        };
        // Ignore error — class may already be registered if the DLL is reloaded.
        let _ = RegisterClassW(&wc);
    });
}

// ── WndProc ───────────────────────────────────────────────────────────────────

unsafe extern "system" fn chart_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            // Suppress the default background erase to prevent flicker.
            // The WM_PAINT handler fills the entire client area itself.
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            if !hdc.is_invalid() {
                unsafe { paint_chart(hdc, hwnd) };
            }
            let _ = unsafe { EndPaint(hwnd, &ps) };
            LRESULT(0)
        }
        WM_SIZE => {
            // Trigger a full repaint when the control is resized.
            let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

// ── Paint ─────────────────────────────────────────────────────────────────────

unsafe fn paint_chart(hdc: windows::Win32::Graphics::Gdi::HDC, hwnd: HWND) {
    let mut rect = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut rect) };

    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;

    // Guard against degenerate sizes during creation / collapse.
    if w <= 0 || h <= 0 {
        return;
    }

    // Plot margins (pixels).
    let margin_left: i32 = 16;
    let margin_right: i32 = 16;
    let margin_top: i32 = 16;
    let margin_bottom: i32 = 16;

    let plot_w = (w - margin_left - margin_right).max(1);
    let plot_h = (h - margin_top - margin_bottom).max(1);

    // 1. Fill dark background.
    let bg_brush = unsafe { CreateSolidBrush(COLOR_BG) };
    let _ = unsafe { FillRect(hdc, &rect, bg_brush) };
    let _ = unsafe { DeleteObject(bg_brush.into()) };

    // 2. Draw five dashed horizontal gridlines.
    let grid_pen = unsafe { CreatePen(PS_DOT, 1, COLOR_GRID) };
    let old_pen = unsafe { SelectObject(hdc, grid_pen.into()) };
    for i in 0i32..=4 {
        let y = margin_top + plot_h * i / 4;
        let _ = unsafe { windows::Win32::Graphics::Gdi::MoveToEx(hdc, margin_left, y, None) };
        let _ = unsafe { windows::Win32::Graphics::Gdi::LineTo(hdc, margin_left + plot_w, y) };
    }
    unsafe { SelectObject(hdc, old_pen) };
    let _ = unsafe { DeleteObject(grid_pen.into()) };

    // 3. Draw hardcoded trend line — 13 weekly data points.
    // Replace this with dynamic data once SetChartData is wired.
    const DATA: [u32; 13] = [1, 3, 5, 2, 4, 7, 3, 8, 5, 6, 9, 4, 7];
    const MAX_VAL: i32 = 10;
    const N: i32 = DATA.len() as i32;

    let mut points = [windows::Win32::Foundation::POINT::default(); 13];
    for (i, &v) in DATA.iter().enumerate() {
        let x = margin_left + plot_w * i as i32 / (N - 1).max(1);
        let y = margin_top + plot_h - (plot_h * v as i32 / MAX_VAL).min(plot_h);
        points[i] = windows::Win32::Foundation::POINT { x, y };
    }

    let line_pen = unsafe { CreatePen(PS_SOLID, 2, COLOR_LINE) };
    let old_pen = unsafe { SelectObject(hdc, line_pen.into()) };
    let _ = unsafe { Polyline(hdc, &points) };
    unsafe { SelectObject(hdc, old_pen) };
    let _ = unsafe { DeleteObject(line_pen.into()) };
}

// ── Command handler ───────────────────────────────────────────────────────────

/*
 * Creates a chart control as a child of `parent_control_id` (or the main window if None).
 * Follows the read-kind-create-hwnd-write pattern used by other control handlers
 * to minimise time spent holding the global window map lock.
 * [CDU-Control-ChartV1] Charts are created exactly once per logical ID and render via WM_PAINT.
 */
pub(crate) fn handle_create_chart_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
) -> PlatformResult<()> {
    log::debug!(
        "ChartHandler: handle_create_chart_command WinID={window_id:?} ParentID={:?} ControlID={}",
        parent_control_id.map(|id| id.raw()),
        control_id.raw()
    );

    // Phase 1: Read-lock pre-checks — verify no duplicate, get parent HWND.
    let parent_hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        if window_data.has_control(control_id) {
            log::warn!(
                "ChartHandler: Chart {} already exists for window {window_id:?}.",
                control_id.raw()
            );
            return Err(PlatformError::OperationFailed(format!(
                "Chart {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        let hwnd_parent = match parent_control_id {
            Some(id) => window_data.get_control_hwnd(id).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "ChartHandler: Parent control {} not found in WinID {window_id:?}",
                    id.raw()
                ))
            })?,
            None => window_data.get_hwnd(),
        };
        if hwnd_parent.is_invalid() {
            return Err(PlatformError::InvalidHandle(format!(
                "ChartHandler: Parent HWND invalid WinID={window_id:?} ParentControlID={parent_control_id:?}",
            )));
        }
        Ok(hwnd_parent)
    })?;

    // Register the custom window class once per process.
    let h_instance = internal_state.h_instance();
    register_chart_class(h_instance);

    // Phase 2: Write-lock to register the control kind.
    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            return Err(PlatformError::OperationFailed(format!(
                "Chart {} already exists for window {window_id:?} (race)",
                control_id.raw()
            )));
        }
        window_data.register_control_kind(control_id, ControlKind::Chart);
        Ok(())
    })?;

    // Phase 3: Create the native HWND outside any lock.
    let hwnd_chart = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CHART_CLASS_NAME,
            &HSTRING::from(""),
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
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
                // Roll back the kind registration on failure.
                let _ = internal_state.with_window_data_write(window_id, |window_data| {
                    window_data.unregister_control_kind(control_id);
                    Ok(())
                });
                return Err(err.into());
            }
        }
    };

    // Phase 4: Write-lock to register the HWND.
    internal_state.with_window_data_write(window_id, |window_data| {
        window_data.register_control_hwnd(control_id, hwnd_chart);
        Ok(())
    })?;

    log::debug!(
        "ChartHandler: chart {} created hwnd={hwnd_chart:?}",
        control_id.raw()
    );
    Ok(())
}
