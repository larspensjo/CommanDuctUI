/*
 * Handles Win32-specific operations for splitter controls.
 * A splitter is a draggable bar that divides two regions (e.g., left/right or top/bottom).
 * It provides mouse capture, cursor management, and emits events during drag operations.
 */

use crate::app::Win32ApiInternalState;
use crate::controls::styling_handler::color_to_colorref;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::types::{ControlId, SplitterOrientation, WindowId};
use crate::window_common::{ControlKind, WM_APP_SPLITTER_DRAG_ENDED, WM_APP_SPLITTER_DRAGGING};

use std::sync::Arc;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT,
        ScreenToClient,
    },
    UI::{
        Input::KeyboardAndMouse::{
            GetCapture, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetCursorPos, GetParent,
            GetWindowLongPtrW, HMENU, IDC_SIZEWE, LoadCursorW, RegisterClassW, SendMessageW,
            SetCursor, SetWindowLongPtrW, WINDOW_EX_STYLE, WM_CANCELMODE, WM_CAPTURECHANGED,
            WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WNDCLASSW,
            WS_CHILD, WS_VISIBLE,
        },
    },
};

// WM_MOUSELEAVE is not exported by windows-rs, define it manually
const WM_MOUSELEAVE: u32 = 0x02A3;
use windows::core::{HSTRING, PCWSTR};

const WC_SPLITTER: PCWSTR = windows::core::w!("CommanductUI_Splitter");

/// Internal state for a single splitter control instance.
/// Stored per-control to track drag and hover state.
#[derive(Debug, Clone)]
pub(crate) struct SplitterInternalState {
    pub orientation: SplitterOrientation,
    pub is_dragging: bool,
    pub is_hovered: bool,
}

impl SplitterInternalState {
    fn new(orientation: SplitterOrientation) -> Self {
        Self {
            orientation,
            is_dragging: false,
            is_hovered: false,
        }
    }
}

// Colors for splitter states
const COLOR_NORMAL: crate::styling::Color = crate::styling::Color {
    r: 0x40,
    g: 0x44,
    b: 0x4B,
};
const COLOR_HOVER: crate::styling::Color = crate::styling::Color {
    r: 0x55,
    g: 0x5A,
    b: 0x64,
};

/// Per-window-instance state stored in GWLP_USERDATA.
/// Used by the splitter's custom WndProc to track hover/drag state.
#[derive(Debug, Default)]
struct SplitterWndData {
    is_hovered: bool,
    is_tracking_mouse: bool,
}

/// Helper to get or create window data from GWLP_USERDATA.
unsafe fn get_wnd_data(hwnd: HWND) -> *mut SplitterWndData {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if ptr == 0 {
            // Allocate and store new window data
            let data = Box::new(SplitterWndData::default());
            let raw = Box::into_raw(data);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
            raw
        } else {
            ptr as *mut SplitterWndData
        }
    }
}

/*
 * Custom window procedure for splitter controls.
 * Handles mouse events for dragging, cursor changes, hover state, and painting.
 */
unsafe extern "system" fn splitter_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_SETCURSOR => {
                // Set the resize cursor when hovering over the splitter
                let cursor = LoadCursorW(None, IDC_SIZEWE).ok();
                SetCursor(cursor);
                return LRESULT(1); // TRUE - we handled it
            }
            WM_LBUTTONDOWN => {
                // Start drag: capture mouse
                SetCapture(hwnd);
                log::debug!("SplitterHandler: Mouse capture started for splitter {hwnd:?}");

                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                let data = get_wnd_data(hwnd);

                // If we have capture, we're dragging - send message to parent
                if GetCapture() == hwnd {
                    if let Ok(parent) = GetParent(hwnd)
                        && !parent.is_invalid()
                    {
                        let mut cursor_pos = windows::Win32::Foundation::POINT::default();
                        if GetCursorPos(&mut cursor_pos).is_ok()
                            && ScreenToClient(parent, &mut cursor_pos).as_bool()
                        {
                            SendMessageW(
                                parent,
                                WM_APP_SPLITTER_DRAGGING,
                                Some(WPARAM(hwnd.0 as usize)),
                                Some(LPARAM(cursor_pos.x as isize)),
                            );
                        }
                    }
                    return LRESULT(0);
                }

                // Not dragging - track hover state
                if !(*data).is_hovered {
                    (*data).is_hovered = true;
                    // Request WM_MOUSELEAVE notification
                    if !(*data).is_tracking_mouse {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        if TrackMouseEvent(&mut tme).is_ok() {
                            (*data).is_tracking_mouse = true;
                        }
                    }
                    // Trigger repaint for hover effect
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                return LRESULT(0);
            }
            WM_MOUSELEAVE => {
                let data = get_wnd_data(hwnd);
                (*data).is_hovered = false;
                (*data).is_tracking_mouse = false;
                // Trigger repaint to remove hover effect
                let _ = InvalidateRect(Some(hwnd), None, false);
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                // End drag: release capture and notify parent
                if GetCapture() == hwnd {
                    let _ = ReleaseCapture();
                    log::debug!("SplitterHandler: Mouse capture released for splitter {hwnd:?}");

                    // Send drag ended message to parent
                    if let Ok(parent) = GetParent(hwnd)
                        && !parent.is_invalid()
                    {
                        let mut cursor_pos = windows::Win32::Foundation::POINT::default();
                        if GetCursorPos(&mut cursor_pos).is_ok()
                            && ScreenToClient(parent, &mut cursor_pos).as_bool()
                        {
                            SendMessageW(
                                parent,
                                WM_APP_SPLITTER_DRAG_ENDED,
                                Some(WPARAM(hwnd.0 as usize)),
                                Some(LPARAM(cursor_pos.x as isize)),
                            );
                        }
                    }
                }
                return LRESULT(0);
            }
            WM_CAPTURECHANGED | WM_CANCELMODE => {
                // Capture was lost (e.g., Alt+Tab, Esc) - cancel drag
                log::debug!("SplitterHandler: Capture lost for splitter {hwnd:?} (msg: {msg})");
                return LRESULT(0);
            }
            WM_PAINT => {
                let data = get_wnd_data(hwnd);
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if !hdc.is_invalid() {
                    // Use hover color when hovered, normal color otherwise
                    let color = if (*data).is_hovered {
                        &COLOR_HOVER
                    } else {
                        &COLOR_NORMAL
                    };
                    let brush = CreateSolidBrush(color_to_colorref(color));
                    FillRect(hdc, &ps.rcPaint, brush);
                    let _ = windows::Win32::Graphics::Gdi::DeleteObject(brush.into());
                    let _ = EndPaint(hwnd, &ps);
                }
                return LRESULT(0);
            }
            windows::Win32::UI::WindowsAndMessaging::WM_DESTROY => {
                // Clean up allocated window data
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                if ptr != 0 {
                    let _ = Box::from_raw(ptr as *mut SplitterWndData);
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                return LRESULT(0);
            }
            _ => {}
        }

        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/*
 * Registers the custom splitter window class.
 * Called once during app initialization.
 */
pub(crate) fn register_splitter_class(
    internal_state: &Arc<Win32ApiInternalState>,
) -> PlatformResult<()> {
    let h_instance = internal_state.h_instance();

    let wc = WNDCLASSW {
        lpfnWndProc: Some(splitter_wnd_proc),
        hInstance: h_instance,
        lpszClassName: WC_SPLITTER,
        hCursor: unsafe { LoadCursorW(None, IDC_SIZEWE).unwrap_or_default() },
        ..Default::default()
    };

    unsafe {
        if RegisterClassW(&wc) == 0 {
            // Class might already be registered, which is okay
            log::debug!(
                "SplitterHandler: Splitter window class already registered or registration failed"
            );
        } else {
            log::debug!("SplitterHandler: Splitter window class registered successfully");
        }
    }

    Ok(())
}

/*
 * Creates a native splitter control and registers it in the window's NativeWindowData.
 */
pub(crate) fn handle_create_splitter_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
    orientation: SplitterOrientation,
) -> PlatformResult<()> {
    log::debug!(
        "SplitterHandler: handle_create_splitter_command for WinID {window_id:?}, ControlID {}, Orientation: {orientation:?}",
        control_id.raw()
    );

    // Ensure the window class is registered
    register_splitter_class(internal_state)?;

    // Phase 1: Read-only pre-checks
    let hwnd_parent_for_creation =
        internal_state.with_window_data_read(window_id, |window_data| {
            if window_data.has_control(control_id) {
                log::warn!(
                    "SplitterHandler: Splitter with ID {} already exists for window {window_id:?}.",
                    control_id.raw()
                );
                return Err(PlatformError::OperationFailed(format!(
                    "Splitter with ID {} already exists for window {window_id:?}",
                    control_id.raw()
                )));
            }

            let hwnd_parent = match parent_control_id {
                Some(id) => window_data.get_control_hwnd(id).ok_or_else(|| {
                    log::warn!(
                        "SplitterHandler: Parent control with ID {} not found for CreateSplitter in WinID {window_id:?}",
                        id.raw()
                    );
                    PlatformError::InvalidHandle(format!(
                        "Parent control with ID {} not found for CreateSplitter in WinID {window_id:?}",
                        id.raw()
                    ))
                })?,
                None => window_data.get_hwnd(),
            };

            if hwnd_parent.is_invalid() {
                log::error!(
                    "SplitterHandler: Parent HWND for CreateSplitter is invalid (WinID: {window_id:?})"
                );
                return Err(PlatformError::InvalidHandle(
                    "Parent HWND for CreateSplitter is invalid".to_string(),
                ));
            }
            Ok(hwnd_parent)
        })?;

    // Register control kind
    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            return Err(PlatformError::OperationFailed(format!(
                "Splitter with ID {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        window_data.register_control_kind(control_id, ControlKind::Splitter);
        Ok(())
    })?;

    // Phase 2: Create the native control without holding any locks
    let h_instance = internal_state.h_instance();
    let hwnd_splitter = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WC_SPLITTER,
            &HSTRING::from(""),
            WS_CHILD | WS_VISIBLE,
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

    // Phase 3: Register the HWND and internal state
    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            log::warn!(
                "SplitterHandler: Control ID {} was created concurrently. Destroying new HWND.",
                control_id.raw()
            );
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd_splitter).ok();
            }
            return Err(PlatformError::OperationFailed(format!(
                "Splitter with ID {} was created concurrently",
                control_id.raw()
            )));
        }

        window_data.register_control_hwnd(control_id, hwnd_splitter);
        window_data.register_splitter_state(control_id, SplitterInternalState::new(orientation));

        log::debug!(
            "SplitterHandler: Created splitter (ID {}) for window {window_id:?} with HWND {hwnd_splitter:?}",
            control_id.raw()
        );
        Ok(())
    })
}

// Note: Drag state tracking is handled internally by the splitter's window procedure.
// The splitter sends WM_APP_SPLITTER_DRAGGING and WM_APP_SPLITTER_DRAG_ENDED messages
// to the parent window, which are then handled in window_common.rs to generate AppEvents.
