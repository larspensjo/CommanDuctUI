/*
 * Provides handling for input (EDIT) controls, specifically for custom
 * background and text colors via `WM_CTLCOLOREDIT`. This handler uses the
 * centralized styling system to look up applied styles and set the
 * appropriate colors and brushes during the control's paint cycle.
 */

use crate::PlatformResult;
use crate::app::Win32ApiInternalState;
use crate::styling::Color;
use crate::types::{ControlId, WindowId};
use std::sync::Arc;
use windows::Win32::{
    Foundation::{COLORREF, HWND, LRESULT},
    Graphics::Gdi::{OPAQUE, SetBkColor, SetBkMode, SetTextColor},
    UI::WindowsAndMessaging::GetDlgCtrlID,
};

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
