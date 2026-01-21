/*
 * Handles Win32-specific operations for "panel" controls. Panels are plain
 * STATIC windows used as lightweight containers for other controls. Each panel
 * installs a forwarding window procedure so that important messages from child
 * controls bubble up to the parent window for centralized handling.
 */

use crate::app::Win32ApiInternalState;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::types::{ControlId, WindowId};
use crate::window_common::WC_STATIC;

use std::sync::Arc;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{
        CallWindowProcW, CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GWLP_WNDPROC, GetParent,
        GetWindowLongPtrW, HMENU, SendMessageW, SetWindowLongPtrW, WINDOW_EX_STYLE, WM_COMMAND,
        WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_NOTIFY, WNDPROC, WS_CHILD, WS_VISIBLE,
    },
};

/*
 * Custom window procedure for panels. It forwards selected messages to the
 * parent window so that controls embedded within the panel behave as if they
 * were direct children of the main window.
 */
unsafe extern "system" fn forwarding_panel_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if (msg == WM_COMMAND
            || msg == WM_CTLCOLOREDIT
            || msg == WM_CTLCOLORSTATIC
            || msg == WM_NOTIFY)
            && let Ok(parent) = GetParent(hwnd)
            && !parent.is_invalid()
        {
            return SendMessageW(parent, msg, Some(wparam), Some(lparam));
        }

        let prev = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if prev != 0 {
            let prev_proc: WNDPROC = std::mem::transmute(prev);
            return CallWindowProcW(prev_proc, hwnd, msg, wparam, lparam);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/*
 * Executes the `CreatePanel` command by creating a STATIC control and
 * registering it within the window's `NativeWindowData`.
 * [CDU-Control-PanelV1] Panels act as logical containers so higher-level layout rules can address them just like any control.
 */
pub(crate) fn handle_create_panel_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    panel_id: ControlId,
) -> PlatformResult<()> {
    log::debug!(
        "PanelHandler: handle_create_panel_command for WinID {window_id:?}, PanelID: {}, ParentControlID: {:?}",
        panel_id.raw(),
        parent_control_id.as_ref().map(|id| id.raw())
    );

    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(panel_id) {
            log::warn!(
                "PanelHandler: Panel with logical ID {} already exists for window {window_id:?}.",
                panel_id.raw()
            );
            return Err(PlatformError::OperationFailed(format!(
                "Panel with logical ID {} already exists for window {window_id:?}",
                panel_id.raw()
            )));
        }

        let hwnd_parent = match parent_control_id {
            Some(id) => window_data.get_control_hwnd(id).ok_or_else(|| {
                log::warn!(
                    "PanelHandler: Parent control with logical ID {} not found for CreatePanel in WinID {window_id:?}",
                    id.raw()
                );
                PlatformError::InvalidHandle(format!(
                    "Parent control with logical ID {} not found for CreatePanel in WinID {window_id:?}",
                    id.raw()
                ))
            })?,
            None => window_data.get_hwnd(),
        };

        if hwnd_parent.is_invalid() {
            log::error!(
                "PanelHandler: Parent HWND for CreatePanel is invalid (WinID: {window_id:?}, ParentControlID: {:?})",
                parent_control_id.as_ref().map(|id| id.raw())
            );
            return Err(PlatformError::InvalidHandle(format!(
                "Parent HWND for CreatePanel is invalid (WinID: {window_id:?}, ParentControlID: {:?})",
                parent_control_id.as_ref().map(|id| id.raw())
            )));
        }

        let h_instance = internal_state.h_instance();
        let hwnd_panel = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                WC_STATIC,
                None,
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                10,
                10,
                Some(hwnd_parent),
                Some(HMENU(panel_id.raw() as *mut _)),
                Some(h_instance),
                None,
            )?
        };

        unsafe {
            #[allow(clippy::fn_to_numeric_cast)]
            let prev = SetWindowLongPtrW(hwnd_panel, GWLP_WNDPROC, forwarding_panel_proc as isize);
            SetWindowLongPtrW(hwnd_panel, GWLP_USERDATA, prev);
        }

        window_data.register_control_hwnd(panel_id, hwnd_panel);
        log::debug!(
            "PanelHandler: Created panel (LogicalID {}) for WinID {window_id:?} with HWND {hwnd_panel:?}",
            panel_id.raw()
        );
        Ok(())
    })
}
