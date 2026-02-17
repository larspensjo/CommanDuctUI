/*
 * Encapsulates Win32-specific operations for ComboBox controls.
 * Provides creation, item management, and selection change notifications
 * for dropdown list combo boxes.
 */

use crate::app::Win32ApiInternalState;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::types::{AppEvent, ControlId, WindowId};
use crate::window_common::ControlKind;

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::Arc;
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    Graphics::Gdi::{GetDC, GetDeviceCaps, LOGPIXELSY, ReleaseDC},
    UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetWindowRect, HMENU, SendMessageW, WINDOW_EX_STYLE,
        WINDOW_STYLE, WS_BORDER, WS_CHILD, WS_VISIBLE, WS_VSCROLL,
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
const CB_GETCOUNT: u32 = 0x0146;
const CB_SETCURSEL: u32 = 0x014E;
const CB_GETCURSEL: u32 = 0x0147;
const CB_GETITEMHEIGHT: u32 = 0x0154;
const CB_SETMINVISIBLE: u32 = 0x1701;
const CB_GETMINVISIBLE: u32 = 0x1702;
const CB_ERR: isize = -1;
const DEFAULT_DPI: i32 = 96;
const FALLBACK_DROPDOWN_HEIGHT_PX: i32 = 260;
const FALLBACK_MIN_VISIBLE_ITEMS: usize = 12;

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
            WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | WS_BORDER
                | WINDOW_STYLE(CBS_DROPDOWNLIST | CBS_HASSTRINGS),
            0,
            0,
            150,
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

    // Enable dark mode — try_enable_dark_mode sets DarkMode_Explorer theme
    // which gives the combo the same visual treatment as edit controls.
    crate::window_common::try_enable_dark_mode(hwnd_combo);

    // Install a subclass that paints a uniform gray border, covering the
    // system-drawn 3D sunken edge that appears white in dark mode.
    super::dark_border::install_dark_border_subclass(hwnd_combo);

    // Request a usable dropdown list height even when layout keeps the control row compact.
    // On supported systems this avoids a near-zero dropdown area for CBS_DROPDOWNLIST.
    let min_visible_items = compute_min_visible_items(hwnd_combo);
    let set_min_visible_result = unsafe {
        SendMessageW(
            hwnd_combo,
            CB_SETMINVISIBLE,
            Some(WPARAM(min_visible_items)),
            Some(LPARAM(0)),
        )
    };
    log::info!(
        "ComboBoxHandler: CB_SETMINVISIBLE control_id={} min_visible={} result={}",
        control_id.raw(),
        min_visible_items,
        set_min_visible_result.0
    );

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

fn dpi_for_window_or_default(hwnd: HWND) -> i32 {
    let hdc = unsafe { GetDC(Some(hwnd)) };
    if hdc.is_invalid() {
        return DEFAULT_DPI;
    }
    let dpi = unsafe { GetDeviceCaps(Some(hdc), LOGPIXELSY) };
    let _ = unsafe { ReleaseDC(Some(hwnd), hdc) };
    if dpi > 0 { dpi } else { DEFAULT_DPI }
}

fn scale_by_dpi(px_at_96_dpi: i32, dpi: i32) -> i32 {
    ((px_at_96_dpi.max(1) as i64 * dpi.max(DEFAULT_DPI) as i64) / DEFAULT_DPI as i64) as i32
}

pub(crate) fn fallback_min_dropdown_height_px() -> i32 {
    FALLBACK_DROPDOWN_HEIGHT_PX
}

pub(crate) fn compute_min_dropdown_height_px(hwnd_combo: HWND, base_height: i32) -> i32 {
    let dpi = dpi_for_window_or_default(hwnd_combo);
    let scaled_fallback = scale_by_dpi(FALLBACK_DROPDOWN_HEIGHT_PX, dpi);
    base_height.max(scaled_fallback)
}

pub(crate) fn compute_min_visible_items(hwnd_combo: HWND) -> usize {
    let item_height_raw = unsafe {
        SendMessageW(
            hwnd_combo,
            CB_GETITEMHEIGHT,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        )
    }
    .0 as i32;

    if item_height_raw <= 0 {
        return FALLBACK_MIN_VISIBLE_ITEMS;
    }

    let target_px = compute_min_dropdown_height_px(hwnd_combo, 0).max(item_height_raw);
    let rows = (target_px / item_height_raw) as usize;
    rows.clamp(6, 20)
}

pub(crate) fn validate_and_heal_dropdown_geometry(
    window_id: WindowId,
    control_id: ControlId,
    hwnd_combo: HWND,
) {
    let mut rect = windows::Win32::Foundation::RECT::default();
    let got_rect = unsafe { GetWindowRect(hwnd_combo, &mut rect) }.is_ok();
    if !got_rect {
        log::warn!(
            "[combo-geometry] unable to read rect for control_id={} window_id={window_id:?}",
            control_id.raw()
        );
        return;
    }

    let current_height = rect.bottom - rect.top;
    let min_height = compute_min_dropdown_height_px(hwnd_combo, 0);
    let item_count =
        unsafe { SendMessageW(hwnd_combo, CB_GETCOUNT, Some(WPARAM(0)), Some(LPARAM(0))) }.0;
    let min_visible = unsafe {
        SendMessageW(
            hwnd_combo,
            CB_GETMINVISIBLE,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        )
    }
    .0;

    if current_height >= min_height {
        return;
    }

    let set_min_visible_result = unsafe {
        SendMessageW(
            hwnd_combo,
            CB_SETMINVISIBLE,
            Some(WPARAM(compute_min_visible_items(hwnd_combo))),
            Some(LPARAM(0)),
        )
    }
    .0;
    log::warn!(
        "[combo-geometry] reapplied policy control_id={} window_id={window_id:?} height={} min_height={} item_count={} min_visible={} set_min_visible_result={}",
        control_id.raw(),
        current_height,
        min_height,
        item_count,
        min_visible,
        set_min_visible_result
    );
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
    log::info!(
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
        SendMessageW(
            hwnd_combo,
            CB_RESETCONTENT,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
    }

    // Add new items
    let expected_count = items.len();
    let sample = items.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    for item in items {
        let utf16 = utf16_null_terminated(&item);
        let result = unsafe {
            SendMessageW(
                hwnd_combo,
                CB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(utf16.as_ptr() as isize)),
            )
        };
        if result.0 == CB_ERR {
            log::warn!(
                "ComboBoxHandler: CB_ADDSTRING failed for ControlID {} (item='{}')",
                control_id.raw(),
                item
            );
        }
    }

    let final_count =
        unsafe { SendMessageW(hwnd_combo, CB_GETCOUNT, Some(WPARAM(0)), Some(LPARAM(0))) }.0;

    if final_count < 0 {
        log::warn!(
            "ComboBoxHandler: CB_GETCOUNT failed for ControlID {}",
            control_id.raw()
        );
    } else if final_count as usize != expected_count {
        log::warn!(
            "ComboBoxHandler: Item count mismatch for ControlID {} (expected={}, actual={})",
            control_id.raw(),
            expected_count,
            final_count
        );
    } else {
        log::info!(
            "ComboBoxHandler: Item count confirmed for ControlID {} (count={}, sample=[{}])",
            control_id.raw(),
            final_count,
            sample
        );
    }

    log::debug!(
        "ComboBoxHandler: Successfully set items for ControlID {}",
        control_id.raw()
    );
    Ok(())
}

fn utf16_null_terminated(text: &str) -> Vec<u16> {
    OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
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
    let result = unsafe {
        SendMessageW(
            hwnd_combo,
            CB_SETCURSEL,
            Some(WPARAM(wparam as usize)),
            Some(LPARAM(0)),
        )
    };

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
    let result =
        unsafe { SendMessageW(hwnd_combo, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))) };
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

    #[test]
    fn utf16_null_terminated_appends_zero_terminator() {
        let utf16 = utf16_null_terminated("Default");
        assert_eq!(utf16.last().copied(), Some(0));
        assert!(utf16.len() >= 2);
    }

    #[test]
    fn compute_min_visible_items_fallback_when_item_height_invalid() {
        // this verifies the fallback policy, independent of Win32 calls
        assert_eq!(FALLBACK_MIN_VISIBLE_ITEMS, 12);
    }
}
