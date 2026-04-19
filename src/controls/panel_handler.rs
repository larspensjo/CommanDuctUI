/*
 * Handles Win32-specific operations for "panel" controls. Panels are plain
 * STATIC windows used as lightweight containers for other controls. Each panel
 * installs a forwarding window procedure so that important messages from child
 * controls bubble up to the parent window for centralized handling.
 */

use crate::app::Win32ApiInternalState;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::ffi_safety;
use crate::window_common::WC_STATIC;
use crate::{
    types::{ControlId, WindowId},
    window_common::ControlKind,
};

use std::sync::Arc;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetParent, HMENU, SendMessageW, WINDOW_EX_STYLE,
            WINDOW_STYLE, WM_COMMAND, WM_COMPAREITEM, WM_CTLCOLORBTN, WM_CTLCOLORDLG,
            WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSCROLLBAR, WM_CTLCOLORSTATIC,
            WM_DELETEITEM, WM_DRAWITEM, WM_HSCROLL, WM_MEASUREITEM, WM_NCDESTROY, WM_NOTIFY,
            WM_PARENTNOTIFY, WM_VSCROLL, WS_CHILD, WS_CLIPCHILDREN, WS_VISIBLE,
        },
    },
};

/// Strongly-typed window style for panels to enforce required flags (correctness-by-construction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PanelWindowStyle(WINDOW_STYLE);

impl PanelWindowStyle {
    /// Base style: must clip children and be a visible child.
    fn base() -> Self {
        Self(WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN)
    }

    /// Extracts the underlying WINDOW_STYLE for CreateWindowExW.
    const fn as_raw(self) -> WINDOW_STYLE {
        self.0
    }
}

/*
 * Custom window procedure for panels. It forwards selected messages to the
 * parent window so that controls embedded within the panel behave as if they
 * were direct children of the main window.
 */
fn is_parent_notification(msg: u32) -> bool {
    matches!(
        msg,
        WM_COMMAND
            | WM_NOTIFY
            | WM_PARENTNOTIFY
            | WM_DRAWITEM
            | WM_MEASUREITEM
            | WM_DELETEITEM
            | WM_COMPAREITEM
            | WM_CTLCOLORBTN
            | WM_CTLCOLOREDIT
            | WM_CTLCOLORSTATIC
            | WM_CTLCOLORLISTBOX
            | WM_CTLCOLORSCROLLBAR
            | WM_CTLCOLORDLG
            | WM_HSCROLL
            | WM_VSCROLL
    )
}

unsafe extern "system" fn forwarding_panel_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    ffi_safety::catch_unwind_ffi(
        "forwarding_panel_proc",
        || unsafe {
            if is_parent_notification(msg)
                && let Ok(parent) = GetParent(hwnd)
                && !parent.is_invalid()
            {
                return SendMessageW(parent, msg, Some(wparam), Some(lparam));
            }
            let result = DefSubclassProc(hwnd, msg, wparam, lparam);
            if msg == WM_NCDESTROY {
                let _ = RemoveWindowSubclass(hwnd, Some(forwarding_panel_proc), PANEL_SUBCLASS_ID);
            }
            result
        },
        || unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    )
}

const PANEL_SUBCLASS_ID: usize = 1;

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

        window_data.register_control_kind(panel_id, ControlKind::Static);

        let h_instance = internal_state.h_instance();
        let hwnd_panel = unsafe {
            match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                WC_STATIC,
                None,
                PanelWindowStyle::base().as_raw(),
                0,
                0,
                10,
                10,
                Some(hwnd_parent),
                Some(HMENU(panel_id.raw() as *mut _)),
                Some(h_instance),
                None,
            ) {
                Ok(hwnd) => hwnd,
                Err(err) => {
                    window_data.unregister_control_kind(panel_id);
                    return Err(err.into());
                }
            }
        };

        unsafe {
            if !SetWindowSubclass(hwnd_panel, Some(forwarding_panel_proc), PANEL_SUBCLASS_ID, 0)
                .as_bool()
            {
                let _ = DestroyWindow(hwnd_panel);
                window_data.unregister_control_kind(panel_id);
                return Err(PlatformError::OperationFailed(format!(
                    "Failed to install panel subclass for control {} in window {window_id:?}",
                    panel_id.raw()
                )));
            }
        }

        window_data.register_control_hwnd(panel_id, hwnd_panel);
        log::debug!(
            "PanelHandler: Created panel (LogicalID {}) for WinID {window_id:?} with HWND {hwnd_panel:?}",
            panel_id.raw()
        );
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_style_clips_children() {
        let style = PanelWindowStyle::base().as_raw();
        assert_ne!(style.0 & WS_CLIPCHILDREN.0, 0);
        assert_ne!(style.0 & WS_CHILD.0, 0);
        assert_ne!(style.0 & WS_VISIBLE.0, 0);
    }

    #[test]
    fn parent_notification_set_is_complete_for_panels() {
        assert!(is_parent_notification(WM_COMMAND));
        assert!(is_parent_notification(WM_NOTIFY));
        assert!(is_parent_notification(WM_PARENTNOTIFY));
        assert!(is_parent_notification(WM_DRAWITEM));
        assert!(is_parent_notification(WM_MEASUREITEM));
        assert!(is_parent_notification(WM_DELETEITEM));
        assert!(is_parent_notification(WM_COMPAREITEM));
        assert!(is_parent_notification(WM_CTLCOLORBTN));
        assert!(is_parent_notification(WM_CTLCOLOREDIT));
        assert!(is_parent_notification(WM_CTLCOLORSTATIC));
        assert!(is_parent_notification(WM_CTLCOLORLISTBOX));
        assert!(is_parent_notification(WM_CTLCOLORSCROLLBAR));
        assert!(is_parent_notification(WM_CTLCOLORDLG));
        assert!(is_parent_notification(WM_HSCROLL));
        assert!(is_parent_notification(WM_VSCROLL));
    }
}
