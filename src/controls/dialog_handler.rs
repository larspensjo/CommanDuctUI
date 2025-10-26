/*
 * This module is responsible for handling platform-specific dialog interactions.
 * It implements the logic to display various dialogs (e.g., file open/save,
 * input, folder picker, profile selection) based on commands received from the
 * application logic. It uses the Win32 API for dialog creation and management,
 * and communicates results back to the application logic via `AppEvent`s.
 */

use crate::app::Win32ApiInternalState;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::types::{AppEvent, MessageSeverity, WindowId};
use crate::window_common;

use std::ffi::{OsString, c_void};
use std::mem::{align_of, size_of};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::Arc;

use windows::{
    Win32::{
        Foundation::{FALSE, HWND, LPARAM, TRUE, WPARAM},
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
        UI::Controls::Dialogs::*,
        UI::Shell::{
            FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
            SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
        },
        UI::WindowsAndMessaging::*,
    },
    core::{HSTRING, PCWSTR},
};

// --- Control IDs for the Profile Selection Dialog ---
const ID_DIALOG_PROFILE_LISTBOX: i32 = 4001;
const ID_DIALOG_PROFILE_PROMPT: i32 = 4002;
const ID_DIALOG_PROFILE_CREATE_NEW_BUTTON: i32 = 4003;

/*
 * Creates a `PathBuf` from a null-terminated or unterminated slice of UTF-16 code units.
 *
 * This utility function is used to convert wide-character string buffers,
 * often received from Win32 API calls (like file dialogs), into Rust's `PathBuf`.
 * It searches for the first null terminator to determine the string's length;
 * if no null terminator is found, the entire buffer is used.
 */
pub(crate) fn pathbuf_from_buf(buffer: &[u16]) -> PathBuf {
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let path_os_string = OsString::from_wide(&buffer[..len]);
    PathBuf::from(path_os_string)
}

/*
 * Retrieves the owner HWND for a given WindowId.
 * This is a helper function that uses the with_window_data_read pattern to
 * safely access the native window handle, encapsulating the locking and
 * error handling logic.
 */
pub(crate) fn get_hwnd_owner(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
) -> PlatformResult<HWND> {
    internal_state.with_window_data_read(window_id, |window_data| {
        let hwnd = window_data.get_hwnd();
        if hwnd.is_invalid() {
            log::warn!("get_hwnd_owner found an invalid HWND for WindowId {window_id:?}");
            return Err(PlatformError::InvalidHandle(format!(
                "HWND for WindowId {window_id:?} is invalid"
            )));
        }
        Ok(hwnd)
    })
}

/*
 * Displays a standard Win32 file dialog (Open or Save As).
 * This is a generic helper function used by `handle_show_open_file_dialog_command`
 * and `handle_show_save_file_dialog_command`. It handles the common setup
 * for `OPENFILENAMEW` and processes the dialog result, then sends an
 * appropriate `AppEvent` constructed by `event_constructor`.
 */
#[allow(clippy::too_many_arguments)]
fn show_common_file_dialog<FDialog, FEvent>(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    title: String,
    default_filename: Option<String>,
    filter_spec: String,
    initial_dir: Option<PathBuf>,
    specific_flags: OPEN_FILENAME_FLAGS,
    dialog_fn: FDialog,
    event_constructor: FEvent,
) -> PlatformResult<()>
where
    FDialog: FnOnce(&mut OPENFILENAMEW) -> windows::core::BOOL,
    FEvent: FnOnce(WindowId, Option<PathBuf>) -> AppEvent,
{
    // Retrieve the owner HWND for the dialog.
    let hwnd_owner = get_hwnd_owner(internal_state, window_id)?;

    // Prepare buffer for the file path.
    let mut file_buffer: Vec<u16> = vec![0; 2048]; // Buffer for the path.
    if let Some(fname) = default_filename {
        if !fname.is_empty() {
            let default_name_utf16: Vec<u16> = fname.encode_utf16().collect();
            let len_to_copy = std::cmp::min(default_name_utf16.len(), file_buffer.len() - 1);
            file_buffer[..len_to_copy].copy_from_slice(&default_name_utf16[..len_to_copy]);
        }
    }

    // Prepare strings for the OPENFILENAMEW struct.
    let title_hstring = HSTRING::from(title);
    let filter_utf16: Vec<u16> = filter_spec.encode_utf16().collect();
    let initial_dir_hstring = initial_dir.map(|p| HSTRING::from(p.to_string_lossy().as_ref()));
    let initial_dir_pcwstr = initial_dir_hstring
        .as_ref()
        .map_or(PCWSTR::null(), |h_str| PCWSTR(h_str.as_ptr()));

    // Initialize OPENFILENAMEW struct.
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd_owner,
        lpstrFile: windows::core::PWSTR(file_buffer.as_mut_ptr()),
        nMaxFile: file_buffer.len() as u32,
        lpstrFilter: PCWSTR(filter_utf16.as_ptr()),
        lpstrTitle: PCWSTR(title_hstring.as_ptr()),
        lpstrInitialDir: initial_dir_pcwstr,
        Flags: OFN_EXPLORER | specific_flags,
        ..Default::default()
    };

    // Call the appropriate dialog function (GetOpenFileNameW or GetSaveFileNameW).
    let dialog_succeeded = dialog_fn(&mut ofn).as_bool();
    let mut path_result: Option<PathBuf> = None;

    if dialog_succeeded {
        path_result = Some(pathbuf_from_buf(&file_buffer));
        log::debug!(
            "DialogHandler: Dialog function succeeded. Path: {:?}",
            path_result.as_ref().unwrap()
        );
    } else {
        let error_code = unsafe { CommDlgExtendedError() };
        if error_code != COMMON_DLG_ERRORS(0) {
            log::error!(
                "DialogHandler: Dialog function failed with error. CommDlgExtendedError: {error_code:?}"
            );
        } else {
            log::debug!("DialogHandler: Dialog cancelled by user (no error).");
        }
    }

    // Construct and send the event to the application logic.
    let event = event_constructor(window_id, path_result);
    internal_state.send_event(event);
    Ok(())
}

/*
 * Handles the `ShowSaveFileDialog` platform command.
 * It uses `show_common_file_dialog` to display a Win32 "Save As" dialog and
 * sends an `AppEvent::FileSaveDialogCompleted` upon completion. This function
 * is called by `Win32ApiInternalState::_execute_platform_command`.
 */
pub(crate) fn handle_show_save_file_dialog_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    title: String,
    default_filename: String,
    filter_spec: String,
    initial_dir: Option<PathBuf>,
) -> PlatformResult<()> {
    show_common_file_dialog(
        internal_state,
        window_id,
        title,
        Some(default_filename),
        filter_spec,
        initial_dir,
        OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT | OFN_NOCHANGEDIR,
        |ofn_ptr| unsafe { GetSaveFileNameW(ofn_ptr) },
        |win_id, res_path| AppEvent::FileSaveDialogCompleted {
            window_id: win_id,
            result: res_path,
        },
    )
}

/*
 * Handles the `ShowOpenFileDialog` platform command.
 * It uses `show_common_file_dialog` to display a Win32 "Open" dialog and
 * sends an `AppEvent::FileOpenProfileDialogCompleted` upon completion. This function
 * is called by `Win32ApiInternalState::_execute_platform_command`.
 */
pub(crate) fn handle_show_open_file_dialog_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    title: String,
    filter_spec: String,
    initial_dir: Option<PathBuf>,
) -> PlatformResult<()> {
    show_common_file_dialog(
        internal_state,
        window_id,
        title,
        None,
        filter_spec,
        initial_dir,
        OFN_PATHMUSTEXIST | OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR,
        |ofn_ptr| unsafe { GetOpenFileNameW(ofn_ptr) },
        |win_id, res_path| AppEvent::FileOpenProfileDialogCompleted {
            window_id: win_id,
            result: res_path,
        },
    )
}

/*
 * Internal data structure passed to the `profile_dialog_proc`.
 * It holds the list of profiles and prompt text to display, and captures
 * the user's choice (selected profile name or request to create a new one).
 */
struct ProfileDialogData {
    // Input to the dialog
    available_profiles: Vec<String>,
    prompt_text: String,
    // Output from the dialog
    selected_profile: Option<String>,
    create_new_pressed: bool,
}

/*
 * Dialog procedure for the custom profile selection dialog.
 * Handles `WM_INITDIALOG` to populate the listbox and `WM_COMMAND` to process
 * button clicks (Select, Cancel, Create New) and listbox double-clicks.
 */
unsafe extern "system" fn profile_dialog_proc(
    hdlg: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match msg {
        WM_INITDIALOG => {
            let dialog_data = unsafe { &*(lparam.0 as *const ProfileDialogData) };
            unsafe { SetWindowLongPtrW(hdlg, GWLP_USERDATA, lparam.0) };

            // Set prompt text
            let h_prompt = HSTRING::from(dialog_data.prompt_text.as_str());
            unsafe {
                SetDlgItemTextW(hdlg, ID_DIALOG_PROFILE_PROMPT, &h_prompt).unwrap_or_default();
            }

            // Populate listbox
            if let Ok(hwnd_listbox) = unsafe { GetDlgItem(Some(hdlg), ID_DIALOG_PROFILE_LISTBOX) } {
                for profile_name in &dialog_data.available_profiles {
                    let h_name = HSTRING::from(profile_name.as_str());
                    unsafe {
                        SendMessageW(
                            hwnd_listbox,
                            LB_ADDSTRING,
                            None,
                            Some(LPARAM(h_name.as_ptr() as isize)),
                        );
                    }
                }
                // Select the first item by default if any exist
                if !dialog_data.available_profiles.is_empty() {
                    unsafe {
                        SendMessageW(hwnd_listbox, LB_SETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)));
                    }
                }
            }
            TRUE.0 as isize
        }
        WM_COMMAND => {
            let command_id = window_common::loword_from_wparam(wparam) as u16;
            let notification_code = window_common::highord_from_wparam(wparam) as u32;

            let dialog_data_ptr =
                unsafe { GetWindowLongPtrW(hdlg, GWLP_USERDATA) } as *mut ProfileDialogData;
            if dialog_data_ptr.is_null() {
                return FALSE.0 as isize;
            }
            let dialog_data = unsafe { &mut *dialog_data_ptr };

            let mut handle_selection = || {
                if let Ok(hwnd_listbox) =
                    unsafe { GetDlgItem(Some(hdlg), ID_DIALOG_PROFILE_LISTBOX) }
                {
                    let selected_idx =
                        unsafe { SendMessageW(hwnd_listbox, LB_GETCURSEL, None, None) }.0 as i32;
                    if selected_idx >= 0 {
                        let text_len = unsafe {
                            SendMessageW(
                                hwnd_listbox,
                                LB_GETTEXTLEN,
                                Some(WPARAM(selected_idx as usize)),
                                None,
                            )
                        }
                        .0 as usize;
                        let mut buffer: Vec<u16> = vec![0; text_len + 1];
                        unsafe {
                            SendMessageW(
                                hwnd_listbox,
                                LB_GETTEXT,
                                Some(WPARAM(selected_idx as usize)),
                                Some(LPARAM(buffer.as_mut_ptr() as isize)),
                            );
                        }
                        dialog_data.selected_profile =
                            Some(String::from_utf16_lossy(&buffer[..text_len]));
                    }
                }
                unsafe { EndDialog(hdlg, IDOK.0 as isize).unwrap_or_default() };
            };

            match command_id {
                x if x == IDOK.0 as u16 => {
                    handle_selection();
                    TRUE.0 as isize
                }
                x if x == IDCANCEL.0 as u16 => {
                    unsafe { EndDialog(hdlg, IDCANCEL.0 as isize).unwrap_or_default() };
                    TRUE.0 as isize
                }
                x if x == ID_DIALOG_PROFILE_CREATE_NEW_BUTTON as u16 => {
                    dialog_data.create_new_pressed = true;
                    unsafe {
                        EndDialog(hdlg, ID_DIALOG_PROFILE_CREATE_NEW_BUTTON as isize)
                            .unwrap_or_default()
                    };
                    TRUE.0 as isize
                }
                x if x == ID_DIALOG_PROFILE_LISTBOX as u16 && notification_code == LBN_DBLCLK => {
                    handle_selection();
                    TRUE.0 as isize
                }
                _ => FALSE.0 as isize,
            }
        }
        _ => FALSE.0 as isize,
    }
}

/*
 * Builds a Win32 dialog template in memory for the profile selection dialog.
 */
fn build_profile_dialog_template(
    template_bytes: &mut Vec<u8>,
    title_str: &str,
) -> PlatformResult<()> {
    let style = DS_CENTER | DS_MODALFRAME | DS_SETFONT;
    let dlg_template = DLGTEMPLATE {
        style: style as u32 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_POPUP.0,
        dwExtendedStyle: 0,
        cdit: 5,
        x: 0,
        y: 0,
        cx: 220,
        cy: 140,
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(dlg_template) as *const [u8; size_of::<DLGTEMPLATE>()])
    });

    push_word(template_bytes, 0);
    push_word(template_bytes, 0);
    push_str_utf16(template_bytes, title_str);
    push_word(template_bytes, 8);
    push_str_utf16(template_bytes, "MS Shell Dlg");

    // Prompt Static Text
    align_to_dword(template_bytes);
    let static_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | window_common::SS_LEFT.0,
        id: ID_DIALOG_PROFILE_PROMPT as u16,
        x: 10,
        y: 10,
        cx: 200,
        cy: 20,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(static_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Static");
    push_word(template_bytes, 0);
    push_word(template_bytes, 0);

    // ListBox
    align_to_dword(template_bytes);
    let listbox_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | WS_VSCROLL.0 | LBS_NOTIFY as u32,
        id: ID_DIALOG_PROFILE_LISTBOX as u16,
        x: 10,
        y: 35,
        cx: 200,
        cy: 70,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(listbox_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "ListBox");
    push_word(template_bytes, 0);
    push_word(template_bytes, 0);

    // "Select" Button
    align_to_dword(template_bytes);
    let ok_button_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | BS_DEFPUSHBUTTON as u32,
        id: IDOK.0 as u16,
        x: 10,
        y: 115,
        cx: 50,
        cy: 14,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(ok_button_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Button");
    push_str_utf16(template_bytes, "Select");
    push_word(template_bytes, 0);

    // "Create New" Button
    align_to_dword(template_bytes);
    let create_button_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | BS_PUSHBUTTON as u32,
        id: ID_DIALOG_PROFILE_CREATE_NEW_BUTTON as u16,
        x: 80,
        y: 115,
        cx: 60,
        cy: 14,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(create_button_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Button");
    push_str_utf16(template_bytes, "Create New...");
    push_word(template_bytes, 0);

    // "Cancel" Button
    align_to_dword(template_bytes);
    let cancel_button_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | BS_PUSHBUTTON as u32,
        id: IDCANCEL.0 as u16,
        x: 160,
        y: 115,
        cx: 50,
        cy: 14,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(cancel_button_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Button");
    push_str_utf16(template_bytes, "Cancel");
    push_word(template_bytes, 0);

    Ok(())
}

/*
 * Handles the `ShowProfileSelectionDialog` platform command.
 * Creates and displays a modal profile selection dialog using a dynamically
 * constructed dialog template. Upon completion, it sends an
 * `AppEvent::ProfileSelectionDialogCompleted` with the user's choice.
 */
pub(crate) fn handle_show_profile_selection_dialog_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    available_profiles: Vec<String>,
    title: String,
    prompt: String,
) -> PlatformResult<()> {
    log::debug!(
        "DialogHandler: Showing Profile Selection Dialog. Title: '{title}', Prompt: '{prompt}'"
    );
    let hwnd_owner = get_hwnd_owner(internal_state, window_id)?;

    let mut dialog_data = ProfileDialogData {
        available_profiles,
        prompt_text: prompt,
        selected_profile: None,
        create_new_pressed: false,
    };

    let mut template_bytes = Vec::<u8>::new();
    build_profile_dialog_template(&mut template_bytes, &title)?;

    let dialog_result = unsafe {
        DialogBoxIndirectParamW(
            Some(internal_state.h_instance()),
            template_bytes.as_ptr() as *const DLGTEMPLATE,
            Some(hwnd_owner),
            Some(profile_dialog_proc),
            LPARAM(&mut dialog_data as *mut _ as isize),
        )
    };

    let event = if dialog_data.create_new_pressed {
        AppEvent::ProfileSelectionDialogCompleted {
            window_id,
            chosen_profile_name: None,
            create_new_requested: true,
            user_cancelled: false,
        }
    } else if dialog_result == IDOK.0 as isize {
        AppEvent::ProfileSelectionDialogCompleted {
            window_id,
            chosen_profile_name: dialog_data.selected_profile,
            create_new_requested: false,
            user_cancelled: false,
        }
    } else {
        AppEvent::ProfileSelectionDialogCompleted {
            window_id,
            chosen_profile_name: None,
            create_new_requested: false,
            user_cancelled: true,
        }
    };

    internal_state.send_event(event);
    Ok(())
}

/*
 * Internal data structure passed to the `input_dialog_proc`.
 */
struct InputDialogData {
    prompt_text: String,
    input_text: String,
    success: bool,
}

/*
 * Holds runtime state for the exclude patterns dialog, including the initial text shown to the
 * user, the final text captured from the edit control, and a flag indicating whether the dialog
 * completed via the OK button.
 */
struct ExcludePatternsDialogData {
    initial_text: String,
    result_text: String,
    saved: bool,
}

// Helper to extract the low word from WPARAM.
fn loword_from_wparam(wparam: WPARAM) -> u16 {
    (wparam.0 & 0xFFFF) as u16
}

/*
 * Dialog procedure for the custom input dialog.
 */
unsafe extern "system" fn input_dialog_proc(
    hdlg: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match msg {
        WM_INITDIALOG => {
            unsafe {
                SetWindowLongPtrW(hdlg, GWLP_USERDATA, lparam.0);
            }
            let dialog_data = unsafe { &*(lparam.0 as *const InputDialogData) };
            let h_prompt = HSTRING::from(dialog_data.prompt_text.as_str());
            unsafe {
                SetDlgItemTextW(
                    hdlg,
                    window_common::ID_DIALOG_INPUT_PROMPT_STATIC,
                    &h_prompt,
                )
                .unwrap_or_default();
            }
            if !dialog_data.input_text.is_empty() {
                let h_edit_text = HSTRING::from(dialog_data.input_text.as_str());
                unsafe {
                    SetDlgItemTextW(hdlg, window_common::ID_DIALOG_INPUT_EDIT, &h_edit_text)
                        .unwrap_or_default();
                }
            }
            TRUE.0 as isize
        }
        WM_COMMAND => {
            let command_id = loword_from_wparam(wparam);
            match command_id {
                x if x == IDOK.0 as u16 => {
                    let dialog_data_ptr =
                        unsafe { GetWindowLongPtrW(hdlg, GWLP_USERDATA) } as *mut InputDialogData;
                    if !dialog_data_ptr.is_null() {
                        let dialog_data = unsafe { &mut *dialog_data_ptr };
                        if let Ok(hwnd_edit_ok) =
                            unsafe { GetDlgItem(Some(hdlg), window_common::ID_DIALOG_INPUT_EDIT) }
                        {
                            let mut buffer: [u16; 256] = [0; 256];
                            let len = unsafe { GetWindowTextW(hwnd_edit_ok, &mut buffer) };
                            dialog_data.input_text = if len > 0 {
                                String::from_utf16_lossy(&buffer[0..len as usize])
                            } else {
                                String::new()
                            };
                        }
                        dialog_data.success = true;
                    }
                    unsafe {
                        EndDialog(hdlg, IDOK.0 as isize).unwrap_or_default();
                    }
                    TRUE.0 as isize
                }
                x if x == IDCANCEL.0 as u16 => {
                    let dialog_data_ptr =
                        unsafe { GetWindowLongPtrW(hdlg, GWLP_USERDATA) } as *mut InputDialogData;
                    if !dialog_data_ptr.is_null() {
                        unsafe { (*dialog_data_ptr).success = false };
                    }
                    unsafe { EndDialog(hdlg, IDCANCEL.0 as isize).unwrap_or_default() };
                    TRUE.0 as isize
                }
                _ => FALSE.0 as isize,
            }
        }
        _ => FALSE.0 as isize,
    }
}

/*
 * Dialog procedure for the exclude patterns editor. Responsible for seeding the multi-line edit
 * control with the current patterns and capturing the updated text when the user confirms changes.
 */
unsafe extern "system" fn exclude_patterns_dialog_proc(
    hdlg: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match msg {
        WM_INITDIALOG => {
            unsafe {
                SetWindowLongPtrW(hdlg, GWLP_USERDATA, lparam.0);
            }
            let dialog_data = unsafe { &*(lparam.0 as *const ExcludePatternsDialogData) };
            let prompt_text =
                HSTRING::from("Enter patterns to exclude (one per line, gitignore syntax).");
            unsafe {
                SetDlgItemTextW(
                    hdlg,
                    window_common::ID_DIALOG_EXCLUDE_PATTERNS_PROMPT_STATIC,
                    &prompt_text,
                )
                .unwrap_or_default();
            }

            let seeded_text = dialog_data
                .initial_text
                .replace("\r\n", "\n")
                .replace('\n', "\r\n");
            if !seeded_text.is_empty() {
                let edit_text = HSTRING::from(seeded_text);
                unsafe {
                    SetDlgItemTextW(
                        hdlg,
                        window_common::ID_DIALOG_EXCLUDE_PATTERNS_EDIT,
                        &edit_text,
                    )
                    .unwrap_or_default();
                }
            }

            TRUE.0 as isize
        }
        WM_COMMAND => {
            let command_id = loword_from_wparam(wparam);
            match command_id {
                x if x == IDOK.0 as u16 => {
                    let dialog_data_ptr = unsafe { GetWindowLongPtrW(hdlg, GWLP_USERDATA) }
                        as *mut ExcludePatternsDialogData;
                    if !dialog_data_ptr.is_null() {
                        if let Ok(edit_hwnd) = unsafe {
                            GetDlgItem(Some(hdlg), window_common::ID_DIALOG_EXCLUDE_PATTERNS_EDIT)
                        } {
                            let text_len = unsafe { GetWindowTextLengthW(edit_hwnd) } as usize;
                            let mut buffer: Vec<u16> = vec![0; text_len + 1];
                            let written =
                                unsafe { GetWindowTextW(edit_hwnd, buffer.as_mut_slice()) };
                            buffer.truncate(written as usize);
                            let mut result = String::from_utf16_lossy(&buffer);
                            result = result.replace("\r\n", "\n");
                            unsafe {
                                (*dialog_data_ptr).result_text = result;
                                (*dialog_data_ptr).saved = true;
                            }
                        }
                    }
                    unsafe { EndDialog(hdlg, IDOK.0 as isize).unwrap_or_default() };
                    TRUE.0 as isize
                }
                x if x == IDCANCEL.0 as u16 => {
                    let dialog_data_ptr = unsafe { GetWindowLongPtrW(hdlg, GWLP_USERDATA) }
                        as *mut ExcludePatternsDialogData;
                    if !dialog_data_ptr.is_null() {
                        unsafe { (*dialog_data_ptr).saved = false };
                    }
                    unsafe { EndDialog(hdlg, IDCANCEL.0 as isize).unwrap_or_default() };
                    TRUE.0 as isize
                }
                _ => FALSE.0 as isize,
            }
        }
        WM_CLOSE => {
            unsafe { EndDialog(hdlg, IDCANCEL.0 as isize).unwrap_or_default() };
            TRUE.0 as isize
        }
        _ => FALSE.0 as isize,
    }
}

// Helper to push a u16 word to a byte vector.
fn push_word(vec: &mut Vec<u8>, word: u16) {
    vec.extend_from_slice(&word.to_le_bytes());
}

// Helper to push a null-terminated UTF-16 string to a byte vector.
fn push_str_utf16(vec: &mut Vec<u8>, s: &str) {
    for c in s.encode_utf16() {
        push_word(vec, c);
    }
    push_word(vec, 0);
}

// Helper to align a byte vector to a DWORD (4-byte) boundary.
fn align_to_dword(vec: &mut Vec<u8>) {
    while vec.len() % align_of::<u32>() != 0 {
        vec.push(0);
    }
}

/*
 * Builds a Win32 dialog template in memory for the input dialog.
 */
fn build_input_dialog_template(
    template_bytes: &mut Vec<u8>,
    title_str: &str,
) -> PlatformResult<()> {
    // DLGTEMPLATE
    let style = DS_CENTER | DS_MODALFRAME | DS_SETFONT;
    let dlg_template = DLGTEMPLATE {
        style: style as u32 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_POPUP.0,
        cdit: 4,
        x: 0,
        y: 0,
        cx: 200,
        cy: 80,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(dlg_template) as *const [u8; size_of::<DLGTEMPLATE>()])
    });
    push_word(template_bytes, 0);
    push_word(template_bytes, 0);
    push_str_utf16(template_bytes, title_str);
    push_word(template_bytes, 8);
    push_str_utf16(template_bytes, "MS Shell Dlg");

    // DLGITEMTEMPLATE for Prompt Static Text
    align_to_dword(template_bytes);
    let static_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | window_common::SS_LEFT.0,
        id: window_common::ID_DIALOG_INPUT_PROMPT_STATIC as u16,
        x: 10,
        y: 10,
        cx: 180,
        cy: 10,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(static_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Static");
    push_word(template_bytes, 0);
    push_word(template_bytes, 0);

    // DLGITEMTEMPLATE for Edit Control
    align_to_dword(template_bytes);
    let edit_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32,
        id: window_common::ID_DIALOG_INPUT_EDIT as u16,
        x: 10,
        y: 25,
        cx: 180,
        cy: 12,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(edit_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Edit");
    push_word(template_bytes, 0);
    push_word(template_bytes, 0);

    // DLGITEMTEMPLATE for OK Button
    align_to_dword(template_bytes);
    let ok_button_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | BS_DEFPUSHBUTTON as u32,
        id: IDOK.0 as u16,
        x: 40,
        y: 50,
        cx: 50,
        cy: 14,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(ok_button_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Button");
    push_str_utf16(template_bytes, "OK");
    push_word(template_bytes, 0);

    // DLGITEMTEMPLATE for Cancel Button
    align_to_dword(template_bytes);
    let cancel_button_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | BS_PUSHBUTTON as u32,
        id: IDCANCEL.0 as u16,
        x: 110,
        y: 50,
        cx: 50,
        cy: 14,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(cancel_button_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Button");
    push_str_utf16(template_bytes, "Cancel");
    push_word(template_bytes, 0);

    Ok(())
}

/*
 * Builds the dialog template used by the exclude patterns editor. The layout contains a descriptive
 * prompt label, a multi-line edit control with a vertical scrollbar, and standard OK/Cancel buttons.
 */
fn build_exclude_patterns_dialog_template(
    template_bytes: &mut Vec<u8>,
    title_str: &str,
) -> PlatformResult<()> {
    let style = DS_CENTER | DS_MODALFRAME | DS_SETFONT;
    let dlg_template = DLGTEMPLATE {
        style: style as u32 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_POPUP.0,
        cdit: 4,
        x: 0,
        y: 0,
        cx: 220,
        cy: 170,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(dlg_template) as *const [u8; size_of::<DLGTEMPLATE>()])
    });
    push_word(template_bytes, 0);
    push_word(template_bytes, 0);
    push_str_utf16(template_bytes, title_str);
    push_word(template_bytes, 8);
    push_str_utf16(template_bytes, "MS Shell Dlg");

    // Prompt label
    align_to_dword(template_bytes);
    let prompt_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | window_common::SS_LEFT.0,
        id: window_common::ID_DIALOG_EXCLUDE_PATTERNS_PROMPT_STATIC as u16,
        x: 10,
        y: 10,
        cx: 200,
        cy: 16,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(prompt_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Static");
    push_word(template_bytes, 0);
    push_word(template_bytes, 0);

    // Multi-line edit control
    align_to_dword(template_bytes);
    let edit_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0
            | WS_VISIBLE.0
            | WS_BORDER.0
            | WS_VSCROLL.0
            | WS_TABSTOP.0
            | ES_MULTILINE as u32
            | ES_AUTOVSCROLL as u32
            | ES_WANTRETURN as u32,
        id: window_common::ID_DIALOG_EXCLUDE_PATTERNS_EDIT as u16,
        x: 10,
        y: 28,
        cx: 200,
        cy: 110,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(edit_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Edit");
    push_word(template_bytes, 0);
    push_word(template_bytes, 0);

    // OK button
    align_to_dword(template_bytes);
    let ok_button_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | BS_DEFPUSHBUTTON as u32,
        id: IDOK.0 as u16,
        x: 50,
        y: 145,
        cx: 60,
        cy: 16,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(ok_button_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Button");
    push_str_utf16(template_bytes, "OK");
    push_word(template_bytes, 0);

    // Cancel button
    align_to_dword(template_bytes);
    let cancel_button_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | BS_PUSHBUTTON as u32,
        id: IDCANCEL.0 as u16,
        x: 120,
        y: 145,
        cx: 60,
        cy: 16,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(cancel_button_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Button");
    push_str_utf16(template_bytes, "Cancel");
    push_word(template_bytes, 0);

    Ok(())
}

/*
 * Handles the `ShowInputDialog` platform command.
 */
pub(crate) fn handle_show_input_dialog_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    title: String,
    prompt: String,
    default_text: Option<String>,
    context_tag: Option<String>,
) -> PlatformResult<()> {
    log::debug!("DialogHandler: Showing Input Dialog. Title: '{title}'");
    let hwnd_owner = get_hwnd_owner(internal_state, window_id)?;

    let mut dialog_data = InputDialogData {
        prompt_text: prompt,
        input_text: default_text.unwrap_or_default(),
        success: false,
    };

    let mut template_bytes = Vec::<u8>::new();
    build_input_dialog_template(&mut template_bytes, &title)?;

    let dialog_result = unsafe {
        DialogBoxIndirectParamW(
            Some(internal_state.h_instance()),
            template_bytes.as_ptr() as *const DLGTEMPLATE,
            Some(hwnd_owner),
            Some(input_dialog_proc),
            LPARAM(&mut dialog_data as *mut _ as isize),
        )
    };

    let final_text_result = if dialog_result != 0 && dialog_data.success {
        Some(dialog_data.input_text)
    } else {
        log::debug!(
            "DialogHandler: Input dialog cancelled or failed. Result: {:?}, Success flag: {}",
            dialog_result,
            dialog_data.success
        );
        None
    };

    let event = AppEvent::GenericInputDialogCompleted {
        window_id,
        text: final_text_result,
        context_tag,
    };

    internal_state.send_event(event);
    Ok(())
}

/*
 * Handles the `ShowExcludePatternsDialog` platform command by displaying a modal dialog that lets
 * the user edit gitignore-style exclude patterns associated with the active profile.
 */
pub(crate) fn handle_show_exclude_patterns_dialog_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    title: String,
    patterns: String,
) -> PlatformResult<()> {
    log::debug!("DialogHandler: Showing Exclude Patterns Dialog. Title: '{title}'.");
    let hwnd_owner = get_hwnd_owner(internal_state, window_id)?;

    let mut dialog_data = ExcludePatternsDialogData {
        initial_text: patterns.replace("\r\n", "\n"),
        result_text: String::new(),
        saved: false,
    };

    let mut template_bytes = Vec::<u8>::new();
    build_exclude_patterns_dialog_template(&mut template_bytes, &title)?;

    let dialog_result = unsafe {
        DialogBoxIndirectParamW(
            Some(internal_state.h_instance()),
            template_bytes.as_ptr() as *const DLGTEMPLATE,
            Some(hwnd_owner),
            Some(exclude_patterns_dialog_proc),
            LPARAM(&mut dialog_data as *mut _ as isize),
        )
    };

    let dialog_saved = dialog_result != 0 && dialog_data.saved;
    let mut initial_text = dialog_data.initial_text;
    let mut result_text = dialog_data.result_text;
    let final_patterns = if dialog_saved {
        std::mem::take(&mut result_text)
    } else {
        std::mem::take(&mut initial_text)
    };

    internal_state.send_event(AppEvent::ExcludePatternsDialogCompleted {
        window_id,
        saved: dialog_saved,
        patterns: final_patterns,
    });
    Ok(())
}

/*
 * Handles the `ShowMessageBox` platform command by displaying a modal Win32 message box.
 * It maps the provided `MessageSeverity` to an icon style, keeping the UX consistent
 * with other status surfaces while ensuring the message appears prominently.
 */
pub(crate) fn handle_show_message_box_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    title: String,
    message: String,
    severity: MessageSeverity,
) -> PlatformResult<()> {
    let hwnd_owner = get_hwnd_owner(internal_state, window_id)?;

    let icon_flag = match severity {
        MessageSeverity::Error => MB_ICONERROR,
        MessageSeverity::Warning => MB_ICONWARNING,
        _ => MB_ICONINFORMATION,
    };

    let title_hstring = HSTRING::from(title);
    let message_hstring = HSTRING::from(message);
    log::debug!(
        "DialogHandler: Showing message box with severity {severity:?} for window {window_id:?}"
    );

    unsafe {
        MessageBoxW(
            Some(hwnd_owner),
            PCWSTR(message_hstring.as_ptr()),
            PCWSTR(title_hstring.as_ptr()),
            MB_OK | icon_flag,
        );
    }

    Ok(())
}

/*
 * Handles the `ShowFolderPickerDialog` platform command.
 */
pub(crate) fn handle_show_folder_picker_dialog_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    title: String,
    initial_dir: Option<PathBuf>,
) -> PlatformResult<()> {
    log::debug!(
        "DialogHandler: Showing real Folder Picker Dialog. Title: '{title}', Initial Dir: {initial_dir:?}"
    );
    let hwnd_owner = get_hwnd_owner(internal_state, window_id)?;
    let mut path_result: Option<PathBuf> = None;

    let file_dialog_result: Result<IFileOpenDialog, windows::core::Error> =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) };

    if let Ok(file_dialog) = file_dialog_result {
        unsafe {
            if let Err(e) = file_dialog.SetOptions(FOS_PICKFOLDERS) {
                log::error!("DialogHandler: IFileOpenDialog::SetOptions failed: {e:?}");
            }
            if let Err(e) = file_dialog.SetTitle(&HSTRING::from(title.as_str())) {
                log::error!("DialogHandler: IFileOpenDialog::SetTitle failed: {e:?}");
            }
            if let Some(dir_path) = &initial_dir {
                if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                    &HSTRING::from(dir_path.as_os_str()),
                    None,
                ) {
                    if let Err(e) = file_dialog.SetDefaultFolder(&item) {
                        log::error!(
                            "DialogHandler: IFileOpenDialog::SetDefaultFolder failed: {e:?}"
                        );
                    }
                }
            }
            if file_dialog.Show(Some(hwnd_owner)).is_ok() {
                if let Ok(shell_item) = file_dialog.GetResult() {
                    if let Ok(pwstr_path) = shell_item.GetDisplayName(SIGDN_FILESYSPATH) {
                        let path_string = pwstr_path.to_string().unwrap_or_default();
                        CoTaskMemFree(Some(pwstr_path.as_ptr() as *const c_void));
                        if !path_string.is_empty() {
                            path_result = Some(PathBuf::from(path_string));
                        }
                    }
                }
            }
        }
    } else if let Err(e) = file_dialog_result {
        let err_msg = format!("DialogHandler: CoCreateInstance failed: {e:?}");
        log::error!("{err_msg}");
        return Err(PlatformError::OperationFailed(err_msg));
    }

    let event = AppEvent::FolderPickerDialogCompleted {
        window_id,
        path: path_result,
    };
    internal_state.send_event(event);
    Ok(())
}
