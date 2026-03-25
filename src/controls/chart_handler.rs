/*
 * Owner-drawn GDI line chart control for the Trends tab.
 *
 * Uses a custom registered window class ("HarvesterChartControl") with its own
 * WndProc so it can respond to WM_PAINT, WM_ERASEBKGND, and WM_SIZE independently
 * of the standard Win32 control classes.
 *
 * Chart data is stored in GWLP_USERDATA as a heap-allocated `ChartWindowState`.
 * The data is replaced by `handle_set_chart_data_command` and freed on WM_DESTROY.
 *
 * Dark theme palette (Win32 COLORREF = 0x00BBGGRR):
 *   Background  #1E2228  → 0x0028_221E
 *   Gridlines   #3A3F47  → 0x0047_3F3A
 */

use crate::app::Win32ApiInternalState;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::types::{ChartDataPacket, ControlId, WindowId};
use crate::window_common::ControlKind;

use std::sync::{Arc, OnceLock};
use windows::Win32::{
    Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM},
    Graphics::Gdi::{
        BACKGROUND_MODE, BeginPaint, CreatePen, CreateSolidBrush, DEFAULT_GUI_FONT, DeleteObject,
        EndPaint, FillRect, GetStockObject, GetTextExtentPoint32W, InvalidateRect, LineTo, MoveToEx,
        PAINTSTRUCT, PS_DOT, PS_SOLID, Polyline, SelectObject, SetBkMode, SetTextColor, TextOutW,
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetClientRect, GetWindowLongPtrW, HMENU,
        RegisterClassW, SetWindowLongPtrW, WINDOW_EX_STYLE, WM_DESTROY, WM_ERASEBKGND, WM_PAINT,
        WM_SIZE, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_VISIBLE,
    },
};
use windows::core::{HSTRING, PCWSTR, w};

// ── Dark theme colors ─────────────────────────────────────────────────────────

const COLOR_BG: COLORREF = COLORREF(0x0028_221E); // #1E2228
const COLOR_GRID: COLORREF = COLORREF(0x0047_3F3A); // #3A3F47

// ── Per-window state stored in GWLP_USERDATA ─────────────────────────────────

struct ChartWindowState {
    data: ChartDataPacket,
}

impl Default for ChartWindowState {
    fn default() -> Self {
        Self {
            data: ChartDataPacket {
                lines: vec![],
                week_labels: vec![],
                is_loading: false,
                show_x_axis_labels: false,
                show_y_axis_labels: false,
                show_end_labels: false,
            },
        }
    }
}

/// Gets or lazily allocates the `ChartWindowState` for this HWND.
/// Mirrors the pattern in `splitter_handler::get_wnd_data`.
unsafe fn get_or_init_chart_state(hwnd: HWND) -> *mut ChartWindowState {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if ptr == 0 {
            let data = Box::new(ChartWindowState::default());
            let raw = Box::into_raw(data);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
            raw
        } else {
            ptr as *mut ChartWindowState
        }
    }
}

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
        WM_DESTROY => {
            // Free the heap-allocated ChartWindowState.
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
            if ptr != 0 {
                let _ = unsafe { Box::from_raw(ptr as *mut ChartWindowState) };
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

// ── Pure helpers ──────────────────────────────────────────────────────────────

/// Returns `tick_count` "nice" rounded tick values starting at 0.
///
/// If `max_val == 0` all ticks are 0. Otherwise finds the smallest value in a
/// fixed nice-number list such that `step * (tick_count - 1) >= max_val` and
/// returns `[0, step, 2*step, ..., (tick_count-1)*step]`.
fn build_y_axis_ticks(max_val: u32, tick_count: usize) -> Vec<u32> {
    if tick_count == 0 {
        return vec![];
    }
    if max_val == 0 {
        return vec![0; tick_count];
    }
    const NICE: [u32; 13] = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000];
    let intervals = (tick_count - 1).max(1) as u32;
    let step = NICE
        .iter()
        .copied()
        .find(|&s| s * intervals >= max_val)
        .unwrap_or_else(|| {
            // Fallback: round up max_val / intervals to nearest nice magnitude
            max_val.div_ceil(intervals) * intervals
        });
    (0..tick_count).map(|i| i as u32 * step).collect()
}

/// Returns the stride (every Nth label to show) so that labels don't overlap.
///
/// Uses a label width estimate of 40 px. Returns 1 when `label_count == 0`.
fn resolve_x_label_stride(plot_width: i32, label_count: usize) -> usize {
    if label_count == 0 {
        return 1;
    }
    let label_width_px: i32 = 40;
    let max_labels = (plot_width / label_width_px).max(1);
    (((label_count as i32 - 1) / max_labels) + 1).max(1) as usize
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

    // Read chart state (null pointer = use default empty state).
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ChartWindowState;
    let default_state = ChartWindowState::default();
    let state: &ChartWindowState = if state_ptr.is_null() {
        &default_state
    } else {
        unsafe { &*state_ptr }
    };
    let lines = &state.data.lines;
    let is_loading = state.data.is_loading;
    let show_x_axis_labels = state.data.show_x_axis_labels;
    let show_y_axis_labels = state.data.show_y_axis_labels;
    let week_labels = state.data.week_labels.clone();

    // Compute max_val for y-axis before layout (needed for margin_left).
    let max_val_u32: u32 = lines
        .iter()
        .flat_map(|l| &l.weekly_counts)
        .copied()
        .max()
        .unwrap_or(0);

    // Select default GUI font early so text measurements are accurate.
    let hfont = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    let old_font = unsafe { SelectObject(hdc, hfont) };
    unsafe { SetBkMode(hdc, BACKGROUND_MODE(1)) }; // TRANSPARENT

    // Plot layout — derive margin_left from measured y-axis label width.
    let margin_left: i32 = if show_y_axis_labels {
        let ticks = build_y_axis_ticks(max_val_u32, 5);
        let widest_label = ticks
            .iter()
            .map(|&t| format!("{t}"))
            .max_by_key(|s| s.len())
            .unwrap_or_default();
        let wide: Vec<u16> = widest_label.encode_utf16().collect();
        let mut sz = SIZE::default();
        if !wide.is_empty() {
            let _ = unsafe { GetTextExtentPoint32W(hdc, &wide, &mut sz) };
        }
        sz.cx + 8
    } else {
        16
    };
    let legend_w: i32 = if lines.is_empty() { 0 } else { 130 };
    let margin_right: i32 = 16 + legend_w;
    let margin_top: i32 = 16;
    let margin_bottom: i32 = if show_x_axis_labels { 20 } else { 16 };

    let plot_w = (w - margin_left - margin_right).max(1);
    let plot_h = (h - margin_top - margin_bottom).max(1);

    // 1. Fill dark background.
    let bg_brush = unsafe { CreateSolidBrush(COLOR_BG) };
    let _ = unsafe { FillRect(hdc, &rect, bg_brush) };
    let _ = unsafe { DeleteObject(bg_brush.into()) };

    // 2. Draw five dashed horizontal gridlines and optional y-axis labels.
    let ticks = if show_y_axis_labels {
        build_y_axis_ticks(max_val_u32, 5)
    } else {
        vec![]
    };
    let grid_pen = unsafe { CreatePen(PS_DOT, 1, COLOR_GRID) };
    let old_pen = unsafe { SelectObject(hdc, grid_pen.into()) };
    for i in 0i32..=4 {
        let y = margin_top + plot_h * i / 4;
        let _ = unsafe { MoveToEx(hdc, margin_left, y, None) };
        let _ = unsafe { LineTo(hdc, margin_left + plot_w, y) };

        if show_y_axis_labels && !ticks.is_empty() {
            // i=0 is the top gridline → highest tick value; i=4 is 0.
            let tick_idx = 4 - i as usize;
            let tick_val = ticks[tick_idx];
            let label = format!("{tick_val}");
            let wide: Vec<u16> = label.encode_utf16().collect();
            let mut sz = SIZE::default();
            let _ = unsafe { GetTextExtentPoint32W(hdc, &wide, &mut sz) };
            // Right-align to margin_left - 4.
            let text_x = margin_left - 4 - sz.cx;
            let text_y = y - sz.cy / 2;
            let _ = unsafe { SetTextColor(hdc, COLORREF(0x0080_8080)) };
            let _ = unsafe { TextOutW(hdc, text_x, text_y, &wide) };
        }
    }
    unsafe { SelectObject(hdc, old_pen) };
    let _ = unsafe { DeleteObject(grid_pen.into()) };

    // 3. Loading placeholder.
    if is_loading {
        let msg: Vec<u16> = "Loading\u{2026}".encode_utf16().collect();
        let _ = unsafe { SetTextColor(hdc, COLORREF(0x0080_8080)) };
        let _ = unsafe { TextOutW(hdc, margin_left, margin_top + plot_h / 2 - 8, &msg) };
        unsafe { SelectObject(hdc, old_font) };
        return;
    }

    // 4. Draw entity lines.
    let n_points = lines.first().map(|l| l.weekly_counts.len()).unwrap_or(0);
    if n_points < 2 {
        unsafe { SelectObject(hdc, old_font) };
        return;
    }

    let max_val = (max_val_u32 as i32).max(1);

    for line in lines {
        let n = line.weekly_counts.len();
        if n < 2 {
            continue;
        }
        let points: Vec<windows::Win32::Foundation::POINT> = (0..n)
            .map(|i| {
                let x = margin_left + plot_w * i as i32 / (n as i32 - 1).max(1);
                let y = margin_top + plot_h
                    - (plot_h * line.weekly_counts[i] as i32 / max_val).min(plot_h);
                windows::Win32::Foundation::POINT { x, y }
            })
            .collect();

        let pen = unsafe { CreatePen(PS_SOLID, 2, COLORREF(line.color)) };
        let old_pen = unsafe { SelectObject(hdc, pen.into()) };
        let _ = unsafe { Polyline(hdc, &points) };
        unsafe { SelectObject(hdc, old_pen) };
        let _ = unsafe { DeleteObject(pen.into()) };
    }

    // 5. X-axis week labels.
    if show_x_axis_labels && !week_labels.is_empty() {
        let stride = resolve_x_label_stride(plot_w, week_labels.len());
        let n = week_labels.len();
        let _ = unsafe { SetTextColor(hdc, COLORREF(0x0080_8080)) };
        for (j, label) in week_labels.iter().enumerate() {
            if j % stride != 0 {
                continue;
            }
            let x_center = margin_left + plot_w * j as i32 / (n as i32 - 1).max(1);
            let wide: Vec<u16> = label.encode_utf16().collect();
            let mut sz = SIZE::default();
            let _ = unsafe { GetTextExtentPoint32W(hdc, &wide, &mut sz) };
            let text_x = x_center - sz.cx / 2;
            let text_y = margin_top + plot_h + 3;
            let _ = unsafe { TextOutW(hdc, text_x, text_y, &wide) };
        }
    }

    // 6. Legend (top-right column).
    if lines.is_empty() {
        unsafe { SelectObject(hdc, old_font) };
        return;
    }
    let legend_x = margin_left + plot_w + 8;

    for (i, line) in lines.iter().enumerate() {
        let y = margin_top + i as i32 * 18;

        // Colored swatch: a short horizontal line.
        let swatch_pen = unsafe { CreatePen(PS_SOLID, 2, COLORREF(line.color)) };
        let old_swatch_pen = unsafe { SelectObject(hdc, swatch_pen.into()) };
        let _ = unsafe { MoveToEx(hdc, legend_x, y + 7, None) };
        let _ = unsafe { LineTo(hdc, legend_x + 15, y + 7) };
        unsafe { SelectObject(hdc, old_swatch_pen) };
        let _ = unsafe { DeleteObject(swatch_pen.into()) };

        // Label text.
        let _ = unsafe { SetTextColor(hdc, COLORREF(line.color)) };
        let label_wide: Vec<u16> = line.label.encode_utf16().collect();
        let _ = unsafe { TextOutW(hdc, legend_x + 19, y, &label_wide) };
    }

    unsafe { SelectObject(hdc, old_font) };
}

// ── Command handlers ──────────────────────────────────────────────────────────

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

/// Stores new chart data in the control's GWLP_USERDATA and triggers a repaint.
pub(crate) fn handle_set_chart_data_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    data: ChartDataPacket,
) -> PlatformResult<()> {
    let hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "SetChartData: control {} not found in window {window_id:?}",
                control_id.raw()
            ))
        })
    })?;

    unsafe {
        let state = get_or_init_chart_state(hwnd);
        (*state).data = data;
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build_y_axis_ticks, resolve_x_label_stride};

    // ── build_y_axis_ticks ────────────────────────────────────────────────────

    #[test]
    fn y_ticks_all_zero_when_max_val_is_zero() {
        let ticks = build_y_axis_ticks(0, 5);
        assert_eq!(ticks, vec![0, 0, 0, 0, 0]);
    }

    #[test]
    fn y_ticks_step_one_for_max_val_one() {
        // step must satisfy step * 4 >= 1 → step=1; ticks = [0,1,2,3,4]
        let ticks = build_y_axis_ticks(1, 5);
        assert_eq!(ticks, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn y_ticks_step_five_for_max_val_ten() {
        // ceil(10/4)=2.5 → first nice number >= 2.5 is 5 (since 2*4=8 < 10)
        // ticks = [0, 5, 10, 15, 20]
        let ticks = build_y_axis_ticks(10, 5);
        assert_eq!(ticks, vec![0, 5, 10, 15, 20]);
    }

    #[test]
    fn y_ticks_step_fifty_for_max_val_hundred() {
        // ceil(100/4)=25 → first nice number >= 25 is 50
        // ticks = [0, 50, 100, 150, 200]
        let ticks = build_y_axis_ticks(100, 5);
        assert_eq!(ticks, vec![0, 50, 100, 150, 200]);
    }

    #[test]
    fn y_ticks_step_two_for_max_val_seven() {
        // ceil(7/4)=1.75 → first nice number >= 1.75 is 2
        // ticks = [0, 2, 4, 6, 8]
        let ticks = build_y_axis_ticks(7, 5);
        assert_eq!(ticks, vec![0, 2, 4, 6, 8]);
    }

    // ── resolve_x_label_stride ────────────────────────────────────────────────

    #[test]
    fn stride_is_one_for_zero_labels() {
        assert_eq!(resolve_x_label_stride(400, 0), 1);
    }

    #[test]
    fn stride_is_one_when_labels_fit() {
        // max_labels = 400/40 = 10; ceil(10/10) = 1
        assert_eq!(resolve_x_label_stride(400, 10), 1);
    }

    #[test]
    fn stride_is_three_for_tight_width() {
        // max_labels = 200/40 = 5; ceil(13/5) = 3
        assert_eq!(resolve_x_label_stride(200, 13), 3);
    }

    #[test]
    fn stride_is_seven_for_very_tight_width() {
        // max_labels = 100/40 = 2; ceil(13/2) = 7
        assert_eq!(resolve_x_label_stride(100, 13), 7);
    }
}
