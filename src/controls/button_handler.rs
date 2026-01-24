/*
 * Encapsulates Win32-specific operations for button controls.
 * Provides creation of push buttons and translation of button click
 * notifications into platform-agnostic `AppEvent`s.
 */

use crate::app::Win32ApiInternalState;
use crate::controls::styling_handler::{color_to_colorref, colorref_to_color};
use crate::error::{PlatformError, Result as PlatformResult};
use crate::styling::Color;
use crate::types::{AppEvent, ControlId, WindowId};
use crate::window_common::ControlKind;

use std::sync::Arc;
use windows::Win32::{
    Foundation::{COLORREF, HWND, LRESULT},
    Graphics::Gdi::{
        COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT, CreateSolidBrush, DT_CENTER, DT_SINGLELINE,
        DT_VCENTER, DeleteObject, DrawFocusRect, DrawTextW, FillRect, GetSysColor, HGDIOBJ,
        InflateRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
    UI::Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED},
    UI::WindowsAndMessaging::{
        BS_PUSHBUTTON, CreateWindowExW, DestroyWindow, GetWindowTextLengthW, GetWindowTextW, HMENU,
        WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD, WS_VISIBLE,
    },
};
use windows::core::{HSTRING, PCWSTR};

const WC_BUTTON: PCWSTR = windows::core::w!("BUTTON");

/*
 * Creates a native push button and registers the resulting HWND in the
 * window's `NativeWindowData`. Fails if the window or control ID are
 * invalid or already in use. This function uses a read-create-write pattern
 * to minimize lock contention on the global window map.
 *
 * First, it acquires a read lock to verify that the control doesn't already
 * exist and to get the parent HWND. Then, it creates the native button
 * control without holding any locks. Finally, it acquires a write lock briefly
 * to register the new control, checking for race conditions.
 * [CDU-Control-ButtonV1][CDU-IdempotentCommandsV1] Push buttons are created exactly once per logical ID and surface clicks via AppEvents.
 */
pub(crate) fn handle_create_button_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
    text: String,
) -> PlatformResult<()> {
    log::debug!(
        "ButtonHandler: handle_create_button_command for WinID {window_id:?}, ParentID {:?}, ControlID {}, Text: '{text}'",
        parent_control_id.as_ref().map(|id| id.raw()),
        control_id.raw()
    );

    // Phase 1: Read-only pre-checks.
    // Get the parent HWND for creation while holding only a read lock.
    let hwnd_parent_for_creation =
        internal_state.with_window_data_read(window_id, |window_data| {
            if window_data.has_control(control_id) {
                log::warn!(
                    "ButtonHandler: Button with ID {} already exists for window {window_id:?}.",
                    control_id.raw()
                );
                return Err(PlatformError::OperationFailed(format!(
                    "Button with ID {} already exists for window {window_id:?}",
                    control_id.raw()
                )));
            }

            let hwnd_parent = match parent_control_id {
                Some(id) => window_data.get_control_hwnd(id).ok_or_else(|| {
                    log::warn!(
                        "ButtonHandler: Parent control with ID {} not found for CreateButton in WinID {window_id:?}",
                        id.raw()
                    );
                    PlatformError::InvalidHandle(format!(
                        "Parent control with ID {} not found for CreateButton in WinID {window_id:?}",
                        id.raw()
                    ))
                })?,
                None => window_data.get_hwnd(),
            };

            if hwnd_parent.is_invalid() {
                log::error!(
                    "ButtonHandler: Parent HWND for CreateButton is invalid (WinID: {window_id:?}, ParentControlID: {:?})",
                    parent_control_id.as_ref().map(|id| id.raw())
                );
                return Err(PlatformError::InvalidHandle(format!(
                    "ButtonHandler: Parent HWND for CreateButton is invalid (WinID: {window_id:?}, ParentControlID: {:?})",
                    parent_control_id.as_ref().map(|id| id.raw())
                )));
            }
            Ok(hwnd_parent)
        })?;

    // Phase 2: Create the native control without holding any locks.
    let h_instance = internal_state.h_instance();
    let hwnd_button = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WC_BUTTON,
            &HSTRING::from(text.as_str()),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_PUSHBUTTON as u32),
            0,
            0,
            10,
            10,
            Some(hwnd_parent_for_creation),
            Some(HMENU(control_id.raw() as *mut _)),
            Some(h_instance),
            None,
        )?
    };

    // Phase 3: Acquire a write lock only to register the new HWND.
    internal_state.with_window_data_write(window_id, |window_data| {
        // Re-check for a race condition where another thread created the control
        // while we were not holding a lock.
        if window_data.has_control(control_id) {
            log::warn!(
                "ButtonHandler: Control ID {} was created concurrently for window {window_id:?}. Destroying new HWND.",
                control_id.raw()
            );
            unsafe {
                // Safely ignore error if window is already gone.
                DestroyWindow(hwnd_button).ok();
            }
            return Err(PlatformError::OperationFailed(format!(
                "Button with ID {} was created concurrently for window {window_id:?}",
                control_id.raw()
            )));
        }

        window_data.register_control_hwnd(control_id, hwnd_button);
        window_data.register_control_kind(control_id, ControlKind::Button);
        log::debug!(
            "ButtonHandler: Created button '{text}' (ID {}) for window {window_id:?} with HWND {hwnd_button:?}",
            control_id.raw()
        );
        Ok(())
    })
}

/*
 * Translates a BN_CLICKED notification into an `AppEvent::ButtonClicked`.
 */
pub(crate) fn handle_bn_clicked(
    window_id: WindowId,
    control_id: ControlId,
    hwnd_control: HWND,
) -> AppEvent {
    // [CDU-Control-ButtonV1] Button notifications re-enter the command pipeline as type-safe events carrying the original ControlId.
    log::debug!(
        "ButtonHandler: BN_CLICKED for ID {} (HWND {hwnd_control:?}) in WinID {window_id:?}",
        control_id.raw()
    );
    AppEvent::ButtonClicked {
        window_id,
        control_id,
    }
}

/*
 * Handles WM_DRAWITEM for owner-drawn buttons.
 * Renders buttons with custom background and text colors from applied styles.
 * Supports disabled state, pressed state, and focus rectangle.
 */
pub(crate) fn handle_wm_drawitem(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    draw_item_struct: *const DRAWITEMSTRUCT,
) -> Option<LRESULT> {
    unsafe {
        if draw_item_struct.is_null() {
            return None;
        }
        let dis = &*draw_item_struct;
        let control_id = ControlId::new(dis.CtlID as i32);

        // Get applied style with fallback to system colors
        let style_id = internal_state
            .with_window_data_read(window_id, |window_data| {
                Ok(window_data.get_style_for_control(control_id))
            })
            .ok()
            .flatten();
        let style = style_id.and_then(|sid| internal_state.get_parsed_style(sid));

        // Resolve colors: style values or system defaults as fallback
        let base_bg = style
            .as_ref()
            .and_then(|s| s.background_color.clone())
            .unwrap_or_else(|| colorref_to_color(COLORREF(GetSysColor(COLOR_BTNFACE))));
        let base_fg = style
            .as_ref()
            .and_then(|s| s.text_color.clone())
            .unwrap_or_else(|| colorref_to_color(COLORREF(GetSysColor(COLOR_BTNTEXT))));

        // Determine final colors based on button state
        let is_disabled = (dis.itemState.0 & ODS_DISABLED.0) != 0;
        let is_pressed = (dis.itemState.0 & ODS_SELECTED.0) != 0;

        let (bg_color, text_color) = if is_disabled {
            // Disabled: use system gray text, keep background
            (
                base_bg,
                colorref_to_color(COLORREF(GetSysColor(COLOR_GRAYTEXT))),
            )
        } else if is_pressed {
            // Pressed: darken background by 20%
            let pressed_bg = Color {
                r: (base_bg.r as u32 * 80 / 100) as u8,
                g: (base_bg.g as u32 * 80 / 100) as u8,
                b: (base_bg.b as u32 * 80 / 100) as u8,
            };
            (pressed_bg, base_fg)
        } else {
            (base_bg, base_fg)
        };

        // Fill background
        let brush = CreateSolidBrush(color_to_colorref(&bg_color));
        FillRect(dis.hDC, &dis.rcItem, brush);
        let _ = DeleteObject(brush.into());

        // Get button text (dynamic length, no hardcoded buffer)
        let text_len = GetWindowTextLengthW(dis.hwndItem);
        let mut text_buf = vec![0u16; (text_len + 1) as usize];
        GetWindowTextW(dis.hwndItem, &mut text_buf);

        // Draw text
        SetTextColor(dis.hDC, color_to_colorref(&text_color));
        SetBkMode(dis.hDC, TRANSPARENT);

        // Apply font if available, saving old font for restoration
        let old_font = style
            .as_ref()
            .and_then(|s| s.font_handle)
            .map(|font| SelectObject(dis.hDC, HGDIOBJ(font.0)));

        let mut rect = dis.rcItem;
        DrawTextW(
            dis.hDC,
            &mut text_buf[..text_len as usize],
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );

        // Restore original font to avoid leaking GDI selection state
        if let Some(prev_font) = old_font {
            SelectObject(dis.hDC, prev_font);
        }

        // Draw focus rectangle (inset by 3px; scale by DPI in future)
        if (dis.itemState.0 & ODS_FOCUS.0) != 0 {
            let mut focus_rect = dis.rcItem;
            let _ = InflateRect(&mut focus_rect, -3, -3);
            let _ = DrawFocusRect(dis.hDC, &focus_rect);
        }

        Some(LRESULT(1)) // TRUE - we handled it
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // [CDU-Control-ButtonV1] BN_CLICKED notifications emit the expected `AppEvent::ButtonClicked`.
    fn bn_clicked_translates_to_app_event() {
        let event = handle_bn_clicked(WindowId(9), ControlId::new(5), HWND::default());
        match event {
            AppEvent::ButtonClicked {
                window_id,
                control_id,
            } => {
                assert_eq!(window_id, WindowId(9));
                assert_eq!(control_id, ControlId::new(5));
            }
            other => panic!("Unexpected event: {other:?}"),
        }
    }
}
