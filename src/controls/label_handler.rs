/*
 * This module is responsible for handling platform-specific logic related to
 * label controls (STATIC controls in Win32). It encapsulates the creation,
 * updating, and custom drawing aspects of labels, making the main command
 * executor and window procedure cleaner.
 *
 * It provides functions to execute label-related PlatformCommands and to handle
 * relevant Win32 messages like WM_CTLCOLORSTATIC for custom appearance. The
 * drawing logic prioritizes the new styling system but falls back to the
 * legacy severity-based coloring for backward compatibility.
 */

use crate::{
    app::Win32ApiInternalState,
    error::{PlatformError, Result as PlatformResult},
    styling::Color,
    types::{ControlId, LabelClass, MessageSeverity, WindowId},
    window_common::{SS_LEFT, WC_STATIC},
};

use std::sync::Arc;
use windows::{
    Win32::{
        Foundation::{COLORREF, GetLastError, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{
            COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor, GetSysColorBrush, HBRUSH, HDC,
            InvalidateRect, SetBkMode, SetTextColor, TRANSPARENT,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, GetDlgCtrlID, GetParent, SendMessageW, SetWindowTextW,
            WINDOW_EX_STYLE, WM_SETFONT, WS_CHILD, WS_VISIBLE,
        },
    },
    core::HSTRING,
};

/*
 * Creates a Win32 COLORREF from the platform-agnostic `Color` struct.
 * Win32 expects colors in BGR format, so this function handles the conversion.
 */
fn color_to_colorref(color: &Color) -> COLORREF {
    COLORREF((color.r as u32) | ((color.g as u32) << 8) | ((color.b as u32) << 16))
}

/*
 * Handles the creation of a native label (STATIC) control.
 * This function takes the necessary parameters to create a label, including its parent,
 * logical ID, initial text, and class. It registers the new label's HWND with the
 * NativeWindowData and sets its initial severity, all within a single write transaction.
 */
pub(crate) fn handle_create_label_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_panel_id: ControlId,
    label_id: ControlId,
    initial_text: String,
    class: LabelClass,
) -> PlatformResult<()> {
    log::debug!(
        "LabelHandler: handle_create_label_command for WinID {window_id:?}, LabelID: {}, ParentPanelID: {}, Text: '{initial_text}', Class: {class:?}",
        label_id.raw(),
        parent_panel_id.raw()
    );

    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(label_id) {
            log::warn!(
                "LabelHandler: Label with logical ID {} already exists for window {window_id:?}.",
                label_id.raw()
            );
            return Err(PlatformError::OperationFailed(format!(
                "Label with logical ID {} already exists for window {window_id:?}",
                label_id.raw()
            )));
        }

        let hwnd_parent_panel = window_data
            .get_control_hwnd(parent_panel_id)
            .ok_or_else(|| {
                log::warn!(
                    "LabelHandler: Parent panel with logical ID {} not found for CreateLabel in WinID {window_id:?}.",
                    parent_panel_id.raw()
                );
                PlatformError::InvalidHandle(format!(
                    "Parent panel with logical ID {} not found for CreateLabel in WinID {window_id:?}",
                    parent_panel_id.raw()
                ))
            })?;

        let h_instance = internal_state.h_instance();
        let hwnd_label = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                WC_STATIC, // Use constant for "STATIC"
                &HSTRING::from(initial_text.as_str()),
                WS_CHILD | WS_VISIBLE | SS_LEFT, // Basic label styles
                0,
                0,
                10,
                10, // Dummy position/size, layout rules will adjust
                Some(hwnd_parent_panel),
                Some(windows::Win32::UI::WindowsAndMessaging::HMENU(
                    label_id.raw() as *mut _,
                )), // Use logical ID for the HMENU
                Some(h_instance),
                None,
            )?
        };

        // Apply custom font if this is a status bar label and font exists
        if class == LabelClass::StatusBar {
            if let Some(h_font) = window_data.get_status_bar_font() {
                if !h_font.is_invalid() {
                    unsafe {
                        SendMessageW(
                            hwnd_label,
                            WM_SETFONT,
                            Some(WPARAM(h_font.0 as usize)),
                            Some(LPARAM(1)),
                        )
                    }; // LPARAM(1) to redraw
                    log::debug!(
                        "LabelHandler: Applied status bar font to label ID {}",
                        label_id.raw()
                    );
                }
            }
        }

        window_data.register_control_hwnd(label_id, hwnd_label);
        window_data.set_label_severity(label_id, MessageSeverity::Information); // Default to Information
        log::debug!(
            "LabelHandler: Created label '{initial_text}' (LogicalID {}) for WinID {window_id:?} with HWND {hwnd_label:?}",
            label_id.raw()
        );
        Ok(())
    })
}

/*
 * Handles the update of a label's text and its associated severity.
 * This function is carefully structured to avoid deadlocks. It first acquires a
 * write lock to update the internal severity state and retrieve the label's HWND.
 * It then RELEASES the lock before making any Win32 API calls that could
 * synchronously dispatch messages (like SetWindowTextW), preventing a re-entrant
 * lock on the same thread.
 */
pub(crate) fn handle_update_label_text_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    label_id: ControlId,
    text: String,
    severity: MessageSeverity,
) -> PlatformResult<()> {
    log::debug!(
        "LabelHandler: handle_update_label_text_command for WinID {window_id:?}, LabelID: {}, Text: '{text}', Severity: {severity:?}",
        label_id.raw()
    );

    let hwnd_label =
        internal_state.with_window_data_write(window_id, |window_data| {
            let hwnd = window_data.get_control_hwnd(label_id).ok_or_else(|| {
                log::warn!(
                    "LabelHandler: Label with logical ID {} not found for UpdateLabelText in WinID {window_id:?}.",
                    label_id.raw()
                );
                PlatformError::InvalidHandle(format!(
                    "Label with logical ID {} not found for UpdateLabelText in WinID {window_id:?}",
                    label_id.raw()
                ))
            })?;

            // Update the severity state.
            window_data.set_label_severity(label_id, severity);

            // Return the HWND for use outside the lock.
            Ok(hwnd)
        })?;

    unsafe {
        if SetWindowTextW(hwnd_label, &HSTRING::from(text.as_str())).is_err() {
            let last_error = GetLastError();
            log::error!(
                "LabelHandler: SetWindowTextW for label ID {} failed: {last_error:?}",
                label_id.raw()
            );
            return Err(PlatformError::OperationFailed(format!(
                "SetWindowTextW for label ID {} failed: {last_error:?}",
                label_id.raw()
            )));
        }
        // Trigger repaint for WM_CTLCOLORSTATIC to apply new severity color
        _ = InvalidateRect(Some(hwnd_label), None, true);
    }
    Ok(())
}

/*
 * Handles the WM_CTLCOLORSTATIC message for label controls.
 *
 * This function is called when a label (STATIC control) is about to be drawn.
 * It uses the new styling system to determine colors and brushes. It checks if
 * a `StyleId` is applied to the control. If so, it uses the text color and
 * background brush from the corresponding `ParsedControlStyle`. If no style is
 * applied, it falls back to the legacy severity-based coloring for the status bar.
 */
pub(crate) fn handle_wm_ctlcolorstatic(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    hdc_static_ctrl: HDC,   // Directly pass HDC from WPARAM
    hwnd_static_ctrl: HWND, // Directly pass HWND from LPARAM
) -> Option<LRESULT> {
    let control_id_raw = unsafe { GetDlgCtrlID(hwnd_static_ctrl) };
    if control_id_raw == 0 {
        return None; // Not a control with an ID, let system handle it.
    }
    let control_id = ControlId::new(control_id_raw);

    let style_result: PlatformResult<Option<LRESULT>> =
        internal_state.with_window_data_read(window_id, |window_data| {
            // --- New Styling System Logic ---
            if let Some(style_id) = window_data.get_style_for_control(control_id) {
                if let Some(style) = internal_state.get_parsed_style(style_id) {
                    // A style is defined for this control. Handle it completely and then return.
                    // Do not fall through to the legacy logic.

                    // Apply text color from the style, if defined.
                    if let Some(color) = &style.text_color {
                        unsafe { SetTextColor(hdc_static_ctrl, color_to_colorref(color)) };
                    }
                    // The background should be transparent to show the parent's color.
                    unsafe { SetBkMode(hdc_static_ctrl, TRANSPARENT) };

                    // If the style itself has a background brush, use it.
                    if let Some(brush) = style.background_brush {
                        return Ok(Some(LRESULT(brush.0 as isize)));
                    }

                    // If the style does not have a background brush, it's a transparent label.
                    // We must return the parent's background brush.
                    if let Ok(parent_hwnd) = unsafe { GetParent(hwnd_static_ctrl) } {
                        if !parent_hwnd.is_invalid() {
                            let parent_id_raw = unsafe { GetDlgCtrlID(parent_hwnd) };
                            if parent_id_raw != 0 {
                                let parent_id = ControlId::new(parent_id_raw);
                                if let Some(parent_style_id) =
                                    window_data.get_style_for_control(parent_id)
                                {
                                    if let Some(parent_style) =
                                        internal_state.get_parsed_style(parent_style_id)
                                    {
                                        if let Some(parent_brush) = parent_style.background_brush {
                                            return Ok(Some(LRESULT(parent_brush.0 as isize)));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Fallback for transparent label if parent brush isn't found: use system window brush.
                    // This is better than falling through to legacy logic which would override the text color.
                    let brush: HBRUSH = unsafe { GetSysColorBrush(COLOR_WINDOW) };
                    return Ok(Some(LRESULT(brush.0 as isize)));
                }
            }

            // --- Fallback to Legacy Severity Logic (for status bar) ---
            if let Some(severity) = window_data.get_label_severity(control_id) {
                let color = match severity {
                    MessageSeverity::Error => COLORREF(0x000000FF), // Red
                    MessageSeverity::Warning => COLORREF(0x0000A5FF), // Orange-ish
                    _ => COLORREF(unsafe { GetSysColor(COLOR_WINDOWTEXT) }),
                };
                unsafe {
                    SetTextColor(hdc_static_ctrl, color);
                    SetBkMode(hdc_static_ctrl, TRANSPARENT);
                    let brush: HBRUSH = GetSysColorBrush(COLOR_WINDOW);
                    return Ok(Some(LRESULT(brush.0 as isize)));
                }
            }
            Ok(None)
        });

    style_result.ok().flatten()
}
