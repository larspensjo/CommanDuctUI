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

    let capped_max = max.max(min).min(i32::MAX as u32);
    let capped_min = min.min(capped_max);

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

    let capped_pos = position.min(i32::MAX as u32);
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
