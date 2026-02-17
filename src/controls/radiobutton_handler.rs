/*
 * Encapsulates Win32-specific operations for RadioButton controls.
 * Provides creation of auto radio buttons with optional group start semantics.
 */

use crate::app::Win32ApiInternalState;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::types::ControlId;
use crate::types::WindowId;
use crate::window_common::ControlKind;

use std::sync::Arc;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_SETCHECK, BS_AUTORADIOBUTTON, CreateWindowExW, DestroyWindow, HMENU, SendMessageW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD, WS_GROUP, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{HSTRING, PCWSTR};

const WC_BUTTON: PCWSTR = windows::core::w!("BUTTON");

/*
 * Creates a native RadioButton (BS_AUTORADIOBUTTON) and registers it.
 * Uses read/write/no-lock/write phase pattern for robustness.
 * [CDU-Control-RadioButtonV1] RadioButtons are created exactly once per logical ID
 * and emit selection events via AppEvents.
 */
pub(crate) fn handle_create_radiobutton_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
    text: String,
    group_start: bool,
) -> PlatformResult<()> {
    log::debug!(
        "RadioButtonHandler: handle_create_radiobutton_command for WinID {window_id:?}, ParentID {:?}, ControlID {}, Text: '{}', GroupStart: {}",
        parent_control_id.as_ref().map(|id| id.raw()),
        control_id.raw(),
        text,
        group_start
    );

    // Phase 1: Read-only pre-checks and get parent HWND
    let hwnd_parent_for_creation =
        internal_state.with_window_data_read(window_id, |window_data| {
            if window_data.has_control(control_id) {
                log::warn!(
                    "RadioButtonHandler: RadioButton with ID {} already exists for window {window_id:?}.",
                    control_id.raw()
                );
                return Err(PlatformError::OperationFailed(format!(
                    "RadioButton with ID {} already exists for window {window_id:?}",
                    control_id.raw()
                )));
            }

            let hwnd_parent = match parent_control_id {
                Some(id) => window_data.get_control_hwnd(id).ok_or_else(|| {
                    log::warn!(
                        "RadioButtonHandler: Parent control with ID {} not found for CreateRadioButton in WinID {window_id:?}",
                        id.raw()
                    );
                    PlatformError::InvalidHandle(format!(
                        "Parent control with ID {} not found for CreateRadioButton in WinID {window_id:?}",
                        id.raw()
                    ))
                })?,
                None => window_data.get_hwnd(),
            };

            if hwnd_parent.is_invalid() {
                log::error!(
                    "RadioButtonHandler: Parent HWND for CreateRadioButton is invalid (WinID: {window_id:?}, ParentControlID: {:?})",
                    parent_control_id.as_ref().map(|id| id.raw())
                );
                return Err(PlatformError::InvalidHandle(format!(
                    "RadioButtonHandler: Parent HWND for CreateRadioButton is invalid (WinID: {window_id:?}, ParentControlID: {:?})",
                    parent_control_id.as_ref().map(|id| id.raw())
                )));
            }
            Ok(hwnd_parent)
        })?;

    // Register ControlKind before creation
    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            log::warn!(
                "RadioButtonHandler: RadioButton with ID {} already exists for window {window_id:?}.",
                control_id.raw()
            );
            return Err(PlatformError::OperationFailed(format!(
                "RadioButton with ID {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        window_data.register_control_kind(control_id, ControlKind::RadioButton);
        Ok(())
    })?;

    // Phase 2: Create the native control without holding any locks
    let h_instance = internal_state.h_instance();
    let style = compute_radiobutton_style(group_start);
    let hwnd_radio = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WC_BUTTON,
            &HSTRING::from(text.as_str()),
            style,
            0,
            0,
            10,
            10,
            Some(hwnd_parent_for_creation),
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

    // Try to enable dark mode best-effort
    crate::window_common::try_enable_dark_mode(hwnd_radio);

    // Phase 3: Register the new HWND
    internal_state.with_window_data_write(window_id, |window_data| {
        // Re-check for race condition
        if window_data.has_control(control_id) {
            log::warn!(
                "RadioButtonHandler: Control ID {} was created concurrently for window {window_id:?}. Destroying new HWND.",
                control_id.raw()
            );
            let _ = unsafe { DestroyWindow(hwnd_radio) };
            return Err(PlatformError::OperationFailed(format!(
                "Control ID {} created concurrently",
                control_id.raw()
            )));
        }

        window_data.register_control_hwnd(control_id, hwnd_radio);
        log::debug!(
            "RadioButtonHandler: Registered RadioButton with ControlID {} and HWND {hwnd_radio:?}",
            control_id.raw()
        );
        Ok(())
    })
}

/*
 * Pure helper for computing RadioButton style flags.
 * Group start buttons get WS_GROUP | WS_TABSTOP for mutual exclusion.
 */
fn compute_radiobutton_style(group_start: bool) -> WINDOW_STYLE {
    let mut style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_AUTORADIOBUTTON as u32);
    if group_start {
        style |= WS_GROUP | WS_TABSTOP;
    }
    style
}

pub(crate) fn handle_set_radiobutton_checked_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    checked: bool,
) -> PlatformResult<()> {
    let hwnd_radio = internal_state.with_window_data_read(window_id, |window_data| {
        let hwnd = window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "RadioButton with ID {} not found for window {window_id:?}",
                control_id.raw()
            ))
        })?;

        let kind = window_data.get_control_kind(control_id);
        if kind != Some(ControlKind::RadioButton) {
            return Err(PlatformError::OperationFailed(format!(
                "Control ID {} is not a RadioButton in window {window_id:?}",
                control_id.raw()
            )));
        }
        Ok(hwnd)
    })?;

    let check_state = win32_check_state(checked);
    unsafe {
        let _ = SendMessageW(
            hwnd_radio,
            BM_SETCHECK,
            Some(WPARAM(check_state)),
            Some(LPARAM(0)),
        );
    }
    Ok(())
}

fn win32_check_state(checked: bool) -> usize {
    if checked { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radiobutton_style_includes_group_flags_when_group_start() {
        let style = compute_radiobutton_style(true);
        let expected =
            WS_CHILD | WS_VISIBLE | WS_GROUP | WS_TABSTOP | WINDOW_STYLE(BS_AUTORADIOBUTTON as u32);
        assert_eq!(style, expected);
    }

    #[test]
    fn radiobutton_style_excludes_group_flags_when_not_group_start() {
        let style = compute_radiobutton_style(false);
        let expected = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_AUTORADIOBUTTON as u32);
        assert_eq!(style, expected);
    }

    #[test]
    fn checked_state_maps_to_expected_win32_constant() {
        assert_eq!(win32_check_state(true), 1);
        assert_eq!(win32_check_state(false), 0);
    }
}
