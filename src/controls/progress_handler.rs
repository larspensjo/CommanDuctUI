use crate::app::Win32ApiInternalState;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::types::{ControlId, WindowId};
use crate::window_common::ControlKind;

use std::sync::Arc;
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    UI::{
        Controls::{PBM_SETPOS, PBM_SETRANGE32, PBS_SMOOTH, PROGRESS_CLASSW},
        WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, HMENU, SendMessageW, WINDOW_EX_STYLE, WINDOW_STYLE,
            WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
        },
    },
};

const DEFAULT_PROGRESS_WIDTH: i32 = 10;
const DEFAULT_PROGRESS_HEIGHT: i32 = 10;

pub(crate) fn handle_create_progress_bar_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
) -> PlatformResult<()> {
    let hwnd_parent = internal_state.with_window_data_read(window_id, |window_data| {
        if window_data.has_control(control_id) {
            return Err(PlatformError::OperationFailed(format!(
                "Progress bar with ID {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }

        match parent_control_id {
            Some(parent_id) => window_data.get_control_hwnd(parent_id).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "Parent control with ID {} not found for progress bar in WinID {window_id:?}",
                    parent_id.raw()
                ))
            }),
            None => Ok(window_data.get_hwnd()),
        }
    })?;

    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            return Err(PlatformError::OperationFailed(format!(
                "Progress bar with ID {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        window_data.register_control_kind(control_id, ControlKind::ProgressBar);
        Ok(())
    })?;

    let hwnd_progress = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PROGRESS_CLASSW,
            None,
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WINDOW_STYLE(PBS_SMOOTH),
            0,
            0,
            DEFAULT_PROGRESS_WIDTH,
            DEFAULT_PROGRESS_HEIGHT,
            Some(hwnd_parent),
            Some(HMENU(control_id.raw() as *mut _)),
            Some(internal_state.h_instance()),
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

    // Apply dark mode theming and border subclass to eliminate the
    // light 3D sunken edge that looks wrong on dark backgrounds.
    crate::window_common::try_enable_dark_mode(hwnd_progress);
    super::dark_border::install_dark_border_subclass(hwnd_progress);

    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            unsafe {
                let _ = DestroyWindow(hwnd_progress);
            }
            return Err(PlatformError::OperationFailed(format!(
                "Progress bar with ID {} was created concurrently for WinID {window_id:?}",
                control_id.raw()
            )));
        }

        window_data.register_control_hwnd(control_id, hwnd_progress);
        Ok(())
    })
}

pub(crate) fn handle_set_progress_bar_range(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    min: u32,
    max: u32,
) -> PlatformResult<()> {
    let hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "Progress bar ID {} not found in WinID {window_id:?}",
                control_id.raw()
            ))
        })
    })?;

    let (capped_min, capped_max) = clamp_progress_range(min, max);

    unsafe {
        SendMessageW(
            hwnd,
            PBM_SETRANGE32,
            Some(WPARAM(capped_min as usize)),
            Some(LPARAM(capped_max as isize)),
        );
    }

    Ok(())
}

pub(crate) fn handle_set_progress_bar_position(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    position: u32,
) -> PlatformResult<()> {
    let hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "Progress bar ID {} not found in WinID {window_id:?}",
                control_id.raw()
            ))
        })
    })?;

    let capped_pos = clamp_progress_position(position);
    unsafe {
        SendMessageW(
            hwnd,
            PBM_SETPOS,
            Some(WPARAM(capped_pos as usize)),
            Some(LPARAM(0)),
        );
    }

    Ok(())
}

/// Clamp a progress-bar range so that `max >= min` and both fit in `i32`.
fn clamp_progress_range(min: u32, max: u32) -> (u32, u32) {
    let capped_max = max.max(min).min(i32::MAX as u32);
    let capped_min = min.min(capped_max);
    (capped_min, capped_max)
}

/// Clamp a progress-bar position to the `i32` range required by `PBM_SETPOS`.
fn clamp_progress_position(position: u32) -> u32 {
    position.min(i32::MAX as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_range_normal() {
        assert_eq!(clamp_progress_range(0, 100), (0, 100));
    }

    #[test]
    fn clamp_range_min_exceeds_max_swaps() {
        // When min > max, capped_max becomes min (via .max(min)), then capped_min stays at min.
        assert_eq!(clamp_progress_range(50, 10), (50, 50));
    }

    #[test]
    fn clamp_range_max_above_i32_max() {
        let big = u32::MAX;
        let (capped_min, capped_max) = clamp_progress_range(0, big);
        assert_eq!(capped_max, i32::MAX as u32);
        assert_eq!(capped_min, 0);
    }

    #[test]
    fn clamp_range_both_above_i32_max() {
        let big = i32::MAX as u32 + 100;
        let (capped_min, capped_max) = clamp_progress_range(big, big);
        assert_eq!(capped_max, i32::MAX as u32);
        assert_eq!(capped_min, i32::MAX as u32);
    }

    #[test]
    fn clamp_position_normal() {
        assert_eq!(clamp_progress_position(42), 42);
    }

    #[test]
    fn clamp_position_above_i32_max() {
        assert_eq!(clamp_progress_position(u32::MAX), i32::MAX as u32);
    }
}
