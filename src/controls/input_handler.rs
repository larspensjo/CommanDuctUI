/*
 * Provides handling for input (EDIT) controls, specifically for custom
 * background and text colors via `WM_CTLCOLOREDIT`. This handler uses the
 * centralized styling system to look up applied styles and set the
 * appropriate colors and brushes during the control's paint cycle.
 */

use crate::app::Win32ApiInternalState;
use crate::styling::Color;
use crate::types::{ControlId, WindowId};
use crate::window_common::WM_APP_INPUT_KEYDOWN;
use crate::{PlatformResult, ffi_safety};
use std::sync::Arc;
use windows::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{OPAQUE, SetBkColor, SetBkMode, SetTextColor},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            GA_ROOT, GetAncestor, GetDlgCtrlID, PostMessageW, WM_GETDLGCODE, WM_KEYDOWN,
            WM_NCDESTROY,
        },
    },
};

const INPUT_KEYDOWN_SUBCLASS_ID: usize = 1;

/// Tells the dialog manager to pass all keys (including Enter) directly
/// to the control so a single-line Edit without `ES_WANTRETURN` doesn't
/// beep when there is no default button in the dialog.
const DLGC_WANTALLKEYS: isize = 0x0004;

unsafe extern "system" fn input_keydown_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    ffi_safety::catch_unwind_ffi(
        "input_keydown_subclass_proc",
        || unsafe {
            match msg {
                WM_KEYDOWN => {
                    let root = GetAncestor(hwnd, GA_ROOT);
                    if !root.is_invalid() {
                        let _ = PostMessageW(
                            Some(root),
                            WM_APP_INPUT_KEYDOWN,
                            WPARAM(hwnd.0 as usize),
                            LPARAM(wparam.0 as isize),
                        );
                    }
                }
                WM_GETDLGCODE => {
                    // Ask the dialog manager not to intercept Enter (and
                    // other keys) — without this, a single-line Edit with no
                    // default button in the dialog beeps on Enter.
                    let base = DefSubclassProc(hwnd, msg, wparam, lparam);
                    return LRESULT(base.0 | DLGC_WANTALLKEYS);
                }
                WM_NCDESTROY => {
                    let _ = RemoveWindowSubclass(
                        hwnd,
                        Some(input_keydown_subclass_proc),
                        INPUT_KEYDOWN_SUBCLASS_ID,
                    );
                }
                _ => {}
            }

            DefSubclassProc(hwnd, msg, wparam, lparam)
        },
        || unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    )
}

pub(crate) fn install_input_keydown_subclass(hwnd: HWND) {
    unsafe {
        if !SetWindowSubclass(
            hwnd,
            Some(input_keydown_subclass_proc),
            INPUT_KEYDOWN_SUBCLASS_ID,
            0,
        )
        .as_bool()
        {
            log::warn!("Failed to install input keydown subclass for hwnd {hwnd:?}.");
        }
    }
}

/*
 * Creates a Win32 COLORREF from the platform-agnostic `Color` struct.
 * Win32 expects colors in BGR format, so this function handles the conversion.
 */
fn color_to_colorref(color: &Color) -> COLORREF {
    COLORREF((color.r as u32) | ((color.g as u32) << 8) | ((color.b as u32) << 16))
}

/*
 * Handles the WM_CTLCOLOREDIT message for input controls.
 *
 * This function is called when an EDIT control is about to be drawn. It queries
 * the new styling system to see if a style has been applied to this specific
 * control. If a style is found, it uses the text color and background brush
 * from the parsed style definition to customize the control's appearance.
 */
pub(crate) fn handle_wm_ctlcoloredit(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    hdc_edit: windows::Win32::Graphics::Gdi::HDC,
    hwnd_edit: HWND,
) -> Option<LRESULT> {
    let control_id_raw = unsafe { GetDlgCtrlID(hwnd_edit) };
    if control_id_raw == 0 {
        return None; // Not a control with an ID, let system handle it.
    }
    let control_id = ControlId::new(control_id_raw);

    let result: PlatformResult<Option<LRESULT>> =
        internal_state.with_window_data_read(window_id, |window_data| {
            if let Some(style_id) = window_data.get_style_for_control(control_id)
                && let Some(style) = internal_state.get_parsed_style(style_id)
            {
                // Apply text color from the style, if defined.
                if let Some(color) = &style.text_color {
                    unsafe { SetTextColor(hdc_edit, color_to_colorref(color)) };
                }
                // Apply background color from the style, if defined.
                if let Some(color) = &style.background_color {
                    unsafe { SetBkColor(hdc_edit, color_to_colorref(color)) };
                }
                unsafe {
                    SetBkMode(hdc_edit, OPAQUE);
                }
                // Return the brush handle for the system to use.
                if let Some(brush) = style.background_brush {
                    return Ok(Some(LRESULT(brush.0 as isize)));
                }
            }
            // No style found or style had no brush, default processing.
            Ok(None)
        });

    result.ok().flatten()
}
