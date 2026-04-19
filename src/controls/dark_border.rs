/*
 * Shared dark-mode border painting for controls that have a system-drawn
 * 3D sunken edge (2px: dark top-left, light bottom-right).
 *
 * Provides a subclass procedure that paints a uniform gray border after
 * WM_PAINT / WM_NCPAINT, and a helper to install it on any HWND.
 */

use crate::ffi_safety;

use windows::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{CreateSolidBrush, DeleteObject, FrameRect, GetWindowDC, ReleaseDC},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{GetWindowRect, WM_NCDESTROY, WM_NCPAINT, WM_PAINT},
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

const DARK_BORDER_SUBCLASS_ID: usize = 1;

/// Subclass window procedure that delegates to the original proc then
/// paints a dark gray border after WM_PAINT / WM_NCPAINT.
unsafe extern "system" fn dark_border_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    ffi_safety::catch_unwind_ffi(
        "dark_border_subclass_proc",
        || unsafe {
            let result = DefSubclassProc(hwnd, msg, wparam, lparam);

            if matches!(msg, WM_PAINT | WM_NCPAINT) {
                paint_dark_border(hwnd);
            }
            if msg == WM_NCDESTROY {
                let _ = RemoveWindowSubclass(
                    hwnd,
                    Some(dark_border_subclass_proc),
                    DARK_BORDER_SUBCLASS_ID,
                );
            }

            result
        },
        || unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    )
}

/// Installs the dark-border subclass on the given control HWND.
pub(crate) fn install_dark_border_subclass(hwnd: HWND) {
    unsafe {
        if !SetWindowSubclass(
            hwnd,
            Some(dark_border_subclass_proc),
            DARK_BORDER_SUBCLASS_ID,
            0,
        )
        .as_bool()
        {
            log::warn!("Failed to install dark-border subclass for hwnd {hwnd:?}.");
        }
    }
}
