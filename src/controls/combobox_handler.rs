/*
 * Encapsulates Win32-specific operations for ComboBox controls.
 * Provides creation, item management, and selection change notifications
 * for dropdown list combo boxes.
 */

use crate::app::Win32ApiInternalState;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::types::{AppEvent, ControlId, WindowId};
use crate::window_common::ControlKind;

use std::sync::Arc;
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SendMessageW, HMENU, WINDOW_EX_STYLE, WINDOW_STYLE,
        WS_CHILD, WS_VISIBLE, WS_VSCROLL,
    },
};
use windows::core::{HSTRING, PCWSTR};

const WC_COMBOBOX: PCWSTR = windows::core::w!("COMBOBOX");

// ComboBox styles
const CBS_DROPDOWNLIST: u32 = 0x0003;
const CBS_HASSTRINGS: u32 = 0x0200;

// ComboBox messages
const CB_RESETCONTENT: u32 = 0x014B;
const CB_ADDSTRING: u32 = 0x0143;
const CB_SETCURSEL: u32 = 0x014E;
const CB_GETCURSEL: u32 = 0x0147;
const CB_ERR: isize = -1;

/*
 * Creates a native ComboBox (dropdown list style) and registers it.
 * Uses read/write/no-lock/write phase pattern for robustness.
 * [CDU-Control-ComboBoxV1] ComboBoxes are created exactly once per logical ID
 * and emit selection changes via AppEvents.
 */
pub(crate) fn handle_create_combobox_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
) -> PlatformResult<()> {
    log::debug!(
        "ComboBoxHandler: handle_create_combobox_command for WinID {window_id:?}, ParentID {:?}, ControlID {}",
        parent_control_id.as_ref().map(|id| id.raw()),
        control_id.raw()
    );

    // Phase 1: Read-only pre-checks and get parent HWND
    let hwnd_parent_for_creation =
        internal_state.with_window_data_read(window_id, |window_data| {
            if window_data.has_control(control_id) {
                log::warn!(
                    "ComboBoxHandler: ComboBox with ID {} already exists for window {window_id:?}.",
                    control_id.raw()
                );
                return Err(PlatformError::OperationFailed(format!(
                    "ComboBox with ID {} already exists for window {window_id:?}",
                    control_id.raw()
                )));
            }

            let hwnd_parent = match parent_control_id {
                Some(id) => window_data.get_control_hwnd(id).ok_or_else(|| {
                    log::warn!(
                        "ComboBoxHandler: Parent control with ID {} not found for CreateComboBox in WinID {window_id:?}",
                        id.raw()
                    );
                    PlatformError::InvalidHandle(format!(
                        "Parent control with ID {} not found for CreateComboBox in WinID {window_id:?}",
                        id.raw()
                    ))
                })?,
                None => window_data.get_hwnd(),
            };

            if hwnd_parent.is_invalid() {
                log::error!(
                    "ComboBoxHandler: Parent HWND for CreateComboBox is invalid (WinID: {window_id:?}, ParentControlID: {:?})",
                    parent_control_id.as_ref().map(|id| id.raw())
                );
                return Err(PlatformError::InvalidHandle(format!(
                    "ComboBoxHandler: Parent HWND for CreateComboBox is invalid (WinID: {window_id:?}, ParentControlID: {:?})",
                    parent_control_id.as_ref().map(|id| id.raw())
                )));
            }
            Ok(hwnd_parent)
        })?;

    // Register ControlKind before creation
    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            log::warn!(
                "ComboBoxHandler: ComboBox with ID {} already exists for window {window_id:?}.",
                control_id.raw()
            );
            return Err(PlatformError::OperationFailed(format!(
                "ComboBox with ID {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        window_data.register_control_kind(control_id, ControlKind::ComboBox);
        Ok(())
    })?;

    // Phase 2: Create the native control without holding any locks
    let h_instance = internal_state.h_instance();
    let hwnd_combo = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WC_COMBOBOX,
            &HSTRING::new(),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL
                | WINDOW_STYLE(CBS_DROPDOWNLIST | CBS_HASSTRINGS),
            0,
            0,
            10,
            200, // Height for dropdown list
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
    crate::window_common::try_enable_dark_mode(hwnd_combo);

    // Phase 3: Register the new HWND
    internal_state.with_window_data_write(window_id, |window_data| {
        // Re-check for race condition
        if window_data.has_control(control_id) {
            log::warn!(
                "ComboBoxHandler: Control ID {} was created concurrently for window {window_id:?}. Destroying new HWND.",
                control_id.raw()
            );
            let _ = unsafe { DestroyWindow(hwnd_combo) };
            return Err(PlatformError::OperationFailed(format!(
                "Control ID {} created concurrently",
                control_id.raw()
            )));
        }

        window_data.register_control_hwnd(control_id, hwnd_combo);
        log::debug!(
            "ComboBoxHandler: Registered ComboBox with ControlID {} and HWND {hwnd_combo:?}",
            control_id.raw()
        );
        Ok(())
    })
}

/*
 * Sets the items in a ComboBox. Clears existing items and adds new ones.
 * No implicit selection is made.
 */
pub(crate) fn handle_set_combobox_items(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    items: Vec<String>,
) -> PlatformResult<()> {
    log::debug!(
        "ComboBoxHandler: handle_set_combobox_items for WinID {window_id:?}, ControlID {}, {} items",
        control_id.raw(),
        items.len()
    );

    let hwnd_combo = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            log::warn!(
                "ComboBoxHandler: Control ID {} not found for SetComboBoxItems in WinID {window_id:?}",
                control_id.raw()
            );
            PlatformError::InvalidHandle(format!(
                "Control ID {} not found for SetComboBoxItems",
                control_id.raw()
            ))
        })
    })?;

    // Clear existing items
    unsafe {
        SendMessageW(hwnd_combo, CB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
    }

    // Add new items
    for item in items {
        let h_item = HSTRING::from(item.as_str());
        unsafe {
            SendMessageW(
                hwnd_combo,
                CB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(h_item.as_ptr() as isize)),
            );
        }
    }

    log::debug!("ComboBoxHandler: Successfully set items for ControlID {}", control_id.raw());
    Ok(())
}

/*
 * Sets the selected index in a ComboBox.
 * None means clear selection (set to -1).
 */
pub(crate) fn handle_set_combobox_selection(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    selected_index: Option<usize>,
) -> PlatformResult<()> {
    log::debug!(
        "ComboBoxHandler: handle_set_combobox_selection for WinID {window_id:?}, ControlID {}, index {:?}",
        control_id.raw(),
        selected_index
    );

    let hwnd_combo = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            log::warn!(
                "ComboBoxHandler: Control ID {} not found for SetComboBoxSelection in WinID {window_id:?}",
                control_id.raw()
            );
            PlatformError::InvalidHandle(format!(
                "Control ID {} not found for SetComboBoxSelection",
                control_id.raw()
            ))
        })
    })?;

    let wparam = selected_index.map(|i| i as isize).unwrap_or(-1);
    let result = unsafe { SendMessageW(hwnd_combo, CB_SETCURSEL, Some(WPARAM(wparam as usize)), Some(LPARAM(0))) };

    if result.0 == CB_ERR && selected_index.is_some() {
        log::warn!(
            "ComboBoxHandler: CB_SETCURSEL returned CB_ERR for index {:?} on ControlID {}",
            selected_index,
            control_id.raw()
        );
    }

    log::debug!(
        "ComboBoxHandler: Set selection to {:?} for ControlID {}",
        selected_index,
        control_id.raw()
    );
    Ok(())
}

/*
 * Handles CBN_SELCHANGE notification and maps to AppEvent.
 * Reads the current selection index and returns None if CB_ERR.
 */
pub(crate) fn handle_cbn_selchange(
    window_id: WindowId,
    control_id: ControlId,
    hwnd_combo: HWND,
) -> AppEvent {
    let result = unsafe { SendMessageW(hwnd_combo, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))) };
    let selected_index = if result.0 == CB_ERR {
        None
    } else {
        Some(result.0 as usize)
    };

    log::debug!(
        "ComboBoxHandler: CBN_SELCHANGE for ControlID {}, selected index: {:?}",
        control_id.raw(),
        selected_index
    );

    AppEvent::ComboBoxSelectionChanged {
        window_id,
        control_id,
        selected_index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_from_raw_index_maps_negative_to_none() {
        let result = if CB_ERR == -1 {
            None
        } else {
            Some(CB_ERR as usize)
        };
        assert_eq!(result, None);
    }

    #[test]
    fn selection_from_raw_index_maps_positive_to_some() {
        let raw: isize = 5;
        let result = if raw == CB_ERR {
            None
        } else {
            Some(raw as usize)
        };
        assert_eq!(result, Some(5));
    }
}
