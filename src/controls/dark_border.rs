/*
 * Shared dark-mode border painting for controls that have a system-drawn
 * 3D sunken edge (2px: dark top-left, light bottom-right).
 *
 * Provides a subclass procedure that paints a uniform gray border after
 * WM_PAINT / WM_NCPAINT, and a helper to install it on any HWND.
 */

use windows::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{CreateSolidBrush, DeleteObject, FrameRect, GetWindowDC, ReleaseDC},
    UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, GWLP_USERDATA, GWLP_WNDPROC, GetWindowLongPtrW,
        GetWindowRect, SetWindowLongPtrW, WM_NCPAINT, WM_PAINT, WNDPROC,
    },
};

/// Gray border color matching the system dark-mode edit/treeview border.
const DARK_BORDER_GRAY: COLORREF = COLORREF(0x60 | (0x60 << 8) | (0x60 << 16));

/// Paints a uniform 2px gray border over the full window rect, covering the
/// system-drawn 3D sunken edge. Uses `GetWindowDC` so both client and
/// non-client areas are covered.
unsafe fn paint_dark_border(hwnd: HWND) {
    unsafe {
        let hdc = GetWindowDC(Some(hwnd));
        if hdc.is_invalid() {
            return;
        }
        let mut wr = RECT::default();
        if GetWindowRect(hwnd, &mut wr).is_ok() {
            let w = wr.right - wr.left;
            let h = wr.bottom - wr.top;
            if w > 2 && h > 2 {
                let brush = CreateSolidBrush(DARK_BORDER_GRAY);
                if !brush.0.is_null() {
                    // Outer 1px border
                    let outer = RECT {
                        left: 0,
                        top: 0,
                        right: w,
                        bottom: h,
                    };
                    FrameRect(hdc, &outer, brush);
                    // Inner 1px border (covers the second edge pixel)
                    let inner = RECT {
                        left: 1,
                        top: 1,
                        right: w - 1,
                        bottom: h - 1,
                    };
                    FrameRect(hdc, &inner, brush);
                    let _ = DeleteObject(brush.into());
                }
            }
        }
        let _ = ReleaseDC(Some(hwnd), hdc);
    }
}

/// Subclass window procedure that delegates to the original proc then
/// paints a dark gray border after WM_PAINT / WM_NCPAINT.
unsafe extern "system" fn dark_border_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let prev = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        let result = if prev != 0 {
            let prev_proc: WNDPROC = std::mem::transmute(prev);
            CallWindowProcW(prev_proc, hwnd, msg, wparam, lparam)
        } else {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        };

        if matches!(msg, WM_PAINT | WM_NCPAINT) {
            paint_dark_border(hwnd);
        }

        result
    }
}

/// Installs the dark-border subclass on the given control HWND.
/// The original window procedure is saved and restored via `GWLP_USERDATA`.
pub(crate) fn install_dark_border_subclass(hwnd: HWND) {
    unsafe {
        #[allow(clippy::fn_to_numeric_cast)]
        let prev = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, dark_border_subclass_proc as isize);
        if prev != 0 {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, prev);
        }
    }
}
