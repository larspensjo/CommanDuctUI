use crate::app::Win32ApiInternalState;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::controls::styling_handler::color_to_colorref;
use crate::types::{ControlId, WindowId};
use crate::window_common::{ControlKind, try_enable_dark_mode};

use std::sync::Arc;
use windows::Win32::Foundation::{COLORREF, LPARAM, WPARAM};
use windows::Win32::UI::Controls::RichEdit::{
    CFM_COLOR, CHARFORMATW, EDITSTREAM, EM_SETBKGNDCOLOR, EM_SETCHARFORMAT, EM_STREAMIN,
    MSFTEDIT_CLASS, SCF_DEFAULT, SF_RTF,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, HMENU,
    SendMessageW, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::HSTRING;

const DEFAULT_RICH_EDIT_WIDTH: i32 = 10;
const DEFAULT_RICH_EDIT_HEIGHT: i32 = 10;

pub(crate) fn handle_create_rich_edit_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
) -> PlatformResult<()> {
    let hwnd_parent = internal_state.with_window_data_read(window_id, |window_data| {
        if window_data.has_control(control_id) {
            return Err(PlatformError::OperationFailed(format!(
                "RichEdit with ID {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }

        match parent_control_id {
            Some(parent_id) => window_data.get_control_hwnd(parent_id).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "Parent control with ID {} not found for RichEdit in WinID {window_id:?}",
                    parent_id.raw()
                ))
            }),
            None => Ok(window_data.get_hwnd()),
        }
    })?;

    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            return Err(PlatformError::OperationFailed(format!(
                "RichEdit with ID {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        window_data.register_control_kind(control_id, ControlKind::RichEdit);
        Ok(())
    })?;

    let hwnd_richedit = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            MSFTEDIT_CLASS,
            &HSTRING::new(),
            WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | WINDOW_STYLE(ES_READONLY as u32)
                | WINDOW_STYLE(ES_MULTILINE as u32)
                | WINDOW_STYLE(ES_AUTOVSCROLL as u32),
            0,
            0,
            DEFAULT_RICH_EDIT_WIDTH,
            DEFAULT_RICH_EDIT_HEIGHT,
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

    if internal_state
        .get_parsed_style(crate::styling::StyleId::MainWindowBackground)
        .is_some()
    {
        try_enable_dark_mode(hwnd_richedit);
    }

    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            unsafe {
                let _ = DestroyWindow(hwnd_richedit);
            }
            return Err(PlatformError::OperationFailed(format!(
                "RichEdit with ID {} was created concurrently for WinID {window_id:?}",
                control_id.raw()
            )));
        }

        window_data.register_control_hwnd(control_id, hwnd_richedit);
        Ok(())
    })
}

pub(crate) fn apply_rich_edit_colors(
    hwnd_richedit: windows::Win32::Foundation::HWND,
    background: Option<COLORREF>,
    foreground: Option<COLORREF>,
) {
    unsafe {
        if let Some(bg) = background {
            let _ = SendMessageW(
                hwnd_richedit,
                EM_SETBKGNDCOLOR,
                Some(WPARAM(0)),
                Some(LPARAM(bg.0 as isize)),
            );
        }

        if let Some(fg) = foreground {
            let mut char_format = CHARFORMATW {
                cbSize: std::mem::size_of::<CHARFORMATW>() as u32,
                dwMask: CFM_COLOR,
                crTextColor: fg,
                ..Default::default()
            };
            let _ = SendMessageW(
                hwnd_richedit,
                EM_SETCHARFORMAT,
                Some(WPARAM(SCF_DEFAULT as usize)),
                Some(LPARAM(&mut char_format as *mut CHARFORMATW as isize)),
            );
        }
    }
}

struct RtfStreamContext<'a> {
    data: &'a [u8],
    position: usize,
}

unsafe extern "system" fn rtf_stream_callback(
    cookie: usize,
    buffer: *mut u8,
    requested_bytes: i32,
    written_bytes: *mut i32,
) -> u32 {
    if cookie == 0 || buffer.is_null() || written_bytes.is_null() || requested_bytes < 0 {
        return 1;
    }

    let context = unsafe { &mut *(cookie as *mut RtfStreamContext<'_>) };
    let remaining = &context.data[context.position..];
    let copy_len = remaining.len().min(requested_bytes as usize);
    unsafe {
        std::ptr::copy_nonoverlapping(remaining.as_ptr(), buffer, copy_len);
        *written_bytes = copy_len as i32;
    }
    context.position += copy_len;
    0
}

pub(crate) fn handle_set_rich_edit_content_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    rtf_text: String,
) -> PlatformResult<()> {
    let hwnd = internal_state.with_window_data_read(window_id, |window_data| {
        window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "RichEdit control ID {} not found in WinID {window_id:?}",
                control_id.raw()
            ))
        })
    })?;

    let mut context = RtfStreamContext {
        data: rtf_text.as_bytes(),
        position: 0,
    };
    let mut edit_stream = EDITSTREAM {
        dwCookie: &mut context as *mut RtfStreamContext<'_> as usize,
        dwError: 0,
        pfnCallback: Some(rtf_stream_callback),
    };

    unsafe {
        let _ = SendMessageW(
            hwnd,
            EM_STREAMIN,
            Some(WPARAM(SF_RTF as usize)),
            Some(LPARAM(&mut edit_stream as *mut EDITSTREAM as isize)),
        );
    }

    if edit_stream.dwError != 0 {
        return Err(PlatformError::OperationFailed(format!(
            "EM_STREAMIN failed with error code {}",
            edit_stream.dwError
        )));
    }

    Ok(())
}

pub(crate) fn style_colors_for_rich_edit(
    parsed_style: &crate::styling::ParsedControlStyle,
) -> (Option<COLORREF>, Option<COLORREF>) {
    let background = parsed_style.background_color.as_ref().map(color_to_colorref);
    let foreground = parsed_style.text_color.as_ref().map(color_to_colorref);
    (background, foreground)
}
