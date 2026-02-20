/*
 * Encapsulates Win32-specific operations for CheckBox controls.
 * Provides creation of BS_AUTOCHECKBOX controls and checked-state management.
 * [CDU-Control-CheckBoxV1] CheckBoxes are created exactly once per logical ID
 * and emit toggle events via AppEvent::CheckBoxToggled.
 */

use crate::app::Win32ApiInternalState;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::types::ControlId;
use crate::types::WindowId;
use crate::window_common::ControlKind;

use std::sync::Arc;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Controls::BST_CHECKED;
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, CreateWindowExW, DestroyWindow, HMENU, SendMessageW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{HSTRING, PCWSTR};

const WC_BUTTON: PCWSTR = windows::core::w!("BUTTON");

/*
 * Creates a native CheckBox (BS_AUTOCHECKBOX) and registers it.
 * Uses read/write/no-lock/write phase pattern for robustness (same as radiobutton_handler).
 */
pub(crate) fn handle_create_checkbox_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
    text: String,
) -> PlatformResult<()> {
    log::debug!(
        "CheckBoxHandler: handle_create_checkbox_command for WinID {window_id:?}, ParentID {:?}, ControlID {}, Text: '{}'",
        parent_control_id.as_ref().map(|id| id.raw()),
        control_id.raw(),
        text,
    );

    // Phase 1: Read-only pre-checks and get parent HWND
    let hwnd_parent_for_creation =
        internal_state.with_window_data_read(window_id, |window_data| {
            if window_data.has_control(control_id) {
                log::warn!(
                    "CheckBoxHandler: CheckBox with ID {} already exists for window {window_id:?}.",
                    control_id.raw()
                );
                return Err(PlatformError::OperationFailed(format!(
                    "CheckBox with ID {} already exists for window {window_id:?}",
                    control_id.raw()
                )));
            }

            let hwnd_parent = match parent_control_id {
                Some(id) => window_data.get_control_hwnd(id).ok_or_else(|| {
                    log::warn!(
                        "CheckBoxHandler: Parent control with ID {} not found for CreateCheckBox in WinID {window_id:?}",
                        id.raw()
                    );
                    PlatformError::InvalidHandle(format!(
                        "Parent control with ID {} not found for CreateCheckBox in WinID {window_id:?}",
                        id.raw()
                    ))
                })?,
                None => window_data.get_hwnd(),
            };

            if hwnd_parent.is_invalid() {
                log::error!(
                    "CheckBoxHandler: Parent HWND for CreateCheckBox is invalid (WinID: {window_id:?}, ParentControlID: {:?})",
                    parent_control_id.as_ref().map(|id| id.raw())
                );
                return Err(PlatformError::InvalidHandle(format!(
                    "CheckBoxHandler: Parent HWND for CreateCheckBox is invalid (WinID: {window_id:?}, ParentControlID: {:?})",
                    parent_control_id.as_ref().map(|id| id.raw())
                )));
            }
            Ok(hwnd_parent)
        })?;

    // Register ControlKind before creation
    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            log::warn!(
                "CheckBoxHandler: CheckBox with ID {} already exists for window {window_id:?}.",
                control_id.raw()
            );
            return Err(PlatformError::OperationFailed(format!(
                "CheckBox with ID {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        window_data.register_control_kind(control_id, ControlKind::CheckBox);
        Ok(())
    })?;

    // Phase 2: Create the native control without holding any locks
    let h_instance = internal_state.h_instance();
    let style = compute_checkbox_style();
    let hwnd_checkbox = unsafe {
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

    // Enable dark mode and force classic rendering so WM_CTLCOLORBTN is delivered.
    crate::window_common::apply_button_dark_mode_classic_render(hwnd_checkbox);

    // Phase 3: Register the new HWND
    internal_state.with_window_data_write(window_id, |window_data| {
        // Re-check for race condition
        if window_data.has_control(control_id) {
            log::warn!(
                "CheckBoxHandler: Control ID {} was created concurrently for window {window_id:?}. Destroying new HWND.",
                control_id.raw()
            );
            let _ = unsafe { DestroyWindow(hwnd_checkbox) };
            return Err(PlatformError::OperationFailed(format!(
                "Control ID {} created concurrently",
                control_id.raw()
            )));
        }

        window_data.register_control_hwnd(control_id, hwnd_checkbox);
        log::debug!(
            "CheckBoxHandler: Registered CheckBox with ControlID {} and HWND {hwnd_checkbox:?}",
            control_id.raw()
        );
        Ok(())
    })
}

pub(crate) fn handle_set_checkbox_checked_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    checked: bool,
) -> PlatformResult<()> {
    let hwnd_checkbox = internal_state.with_window_data_read(window_id, |window_data| {
        let hwnd = window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "CheckBox with ID {} not found for window {window_id:?}",
                control_id.raw()
            ))
        })?;

        let kind = window_data.get_control_kind(control_id);
        if kind != Some(ControlKind::CheckBox) {
            return Err(PlatformError::OperationFailed(format!(
                "Control ID {} is not a CheckBox in window {window_id:?}",
                control_id.raw()
            )));
        }
        Ok(hwnd)
    })?;

    let check_state = win32_check_state(checked);
    unsafe {
        let _ = SendMessageW(
            hwnd_checkbox,
            BM_SETCHECK,
            Some(WPARAM(check_state)),
            Some(LPARAM(0)),
        );
    }
    Ok(())
}

/// Reads the current check state of a CheckBox HWND via BM_GETCHECK.
/// Returns `true` if the checkbox is checked (BST_CHECKED), `false` otherwise.
pub(crate) fn read_checkbox_state(hwnd: windows::Win32::Foundation::HWND) -> bool {
    let result = unsafe { SendMessageW(hwnd, BM_GETCHECK, None, None) };
    result.0 as u32 == BST_CHECKED.0
}

/*
 * Pure helper for computing CheckBox style flags.
 * BS_AUTOCHECKBOX handles toggling automatically; WS_TABSTOP makes it keyboard-accessible.
 */
fn compute_checkbox_style() -> WINDOW_STYLE {
    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32)
}

fn win32_check_state(checked: bool) -> usize {
    if checked { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkbox_style_includes_autocheckbox() {
        let style = compute_checkbox_style();
        assert!(
            style.0 & (BS_AUTOCHECKBOX as u32) != 0,
            "style must include BS_AUTOCHECKBOX"
        );
    }

    #[test]
    fn checkbox_style_includes_tabstop() {
        let style = compute_checkbox_style();
        assert!(style.0 & WS_TABSTOP.0 != 0, "style must include WS_TABSTOP");
    }

    #[test]
    fn checked_state_maps_to_expected_win32_constant() {
        assert_eq!(win32_check_state(true), 1);
        assert_eq!(win32_check_state(false), 0);
    }
}
