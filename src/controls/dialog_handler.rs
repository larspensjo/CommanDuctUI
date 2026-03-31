/*
 * This module is responsible for handling platform-specific dialog interactions.
 * It implements the logic to display various dialogs (e.g., file open/save,
 * input, folder picker, profile selection) based on commands received from the
 * application logic. It uses the Win32 API for dialog creation and management,
 * and communicates results back to the application logic via `AppEvent`s.
 */

use crate::app::Win32ApiInternalState;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::types::{
    AppEvent, FormButtons, FormDialogDescriptor, FormField, FormFieldValue, FormFileExistsWarning,
    FormRow, FormTextValidation, MessageSeverity, WindowId,
};
use crate::window_common;

use std::collections::HashMap;
use std::ffi::{OsString, c_void};
use std::mem::{align_of, size_of};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use windows::{
    Win32::{
        Foundation::{COLORREF, FALSE, HWND, LPARAM, TRUE, WPARAM},
        Graphics::Gdi::{
            CreateSolidBrush, HBRUSH, HDC, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
        },
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
        UI::Controls::BST_CHECKED,
        UI::Controls::Dialogs::*,
        UI::Input::KeyboardAndMouse::EnableWindow,
        UI::Shell::{
            FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
            SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
        },
        UI::WindowsAndMessaging::*,
        UI::WindowsAndMessaging::{BM_GETCHECK, EN_CHANGE},
    },
    core::{HSTRING, PCWSTR},
};

// --- Control IDs for the Profile Selection Dialog ---
const ID_DIALOG_PROFILE_LISTBOX: i32 = 4001;
const ID_DIALOG_PROFILE_PROMPT: i32 = 4002;
const ID_DIALOG_PROFILE_CREATE_NEW_BUTTON: i32 = 4003;

// --- Control IDs for the generic form dialog ---
const ID_DIALOG_FORM_FIRST_ROW: i32 = 5001;
const ID_DIALOG_FORM_FIRST_FIELD_LABEL: i32 = 5100;
const ID_DIALOG_FORM_FIRST_FIELD_EDIT: i32 = 5200;
const ID_DIALOG_FORM_FIRST_FIELD_CHECKBOX: i32 = 5300;
const ID_DIALOG_FORM_FIRST_WARNING: i32 = 5400;
const ID_DIALOG_FORM_OK: i32 = 5500;
const ID_DIALOG_FORM_CANCEL: i32 = 5501;

const COLOR_DIALOG_BG: COLORREF = COLORREF(0x0028_221E); // #1E2228
const COLOR_DIALOG_TEXT: COLORREF = COLORREF(0x00E0_E0E0);
const COLOR_DIALOG_WARNING: COLORREF = COLORREF(0x0000_C8FF);

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
 * [CDU-Dialogs-FileV1] Both Open and Save dialog commands flow through this helper so `AppEvent`s always contain a normalized `PathBuf`.
 */
fn show_common_file_dialog<FDialog, FEvent>(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    request: CommonFileDialogRequest,
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
    if let Some(fname) = request.default_filename
        && !fname.is_empty()
    {
        let default_name_utf16: Vec<u16> = fname.encode_utf16().collect();
        let len_to_copy = std::cmp::min(default_name_utf16.len(), file_buffer.len() - 1);
        file_buffer[..len_to_copy].copy_from_slice(&default_name_utf16[..len_to_copy]);
    }

    // Prepare strings for the OPENFILENAMEW struct.
    let title_hstring = HSTRING::from(request.title);
    let filter_utf16: Vec<u16> = request.filter_spec.encode_utf16().collect();
    let initial_dir_hstring = request
        .initial_dir
        .map(|p| HSTRING::from(p.to_string_lossy().as_ref()));
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
        Flags: OFN_EXPLORER | request.specific_flags,
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

struct CommonFileDialogRequest {
    title: String,
    default_filename: Option<String>,
    filter_spec: String,
    initial_dir: Option<PathBuf>,
    specific_flags: OPEN_FILENAME_FLAGS,
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
        CommonFileDialogRequest {
            title,
            default_filename: Some(default_filename),
            filter_spec,
            initial_dir,
            specific_flags: OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT | OFN_NOCHANGEDIR,
        },
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
        CommonFileDialogRequest {
            title,
            default_filename: None,
            filter_spec,
            initial_dir,
            specific_flags: OFN_PATHMUSTEXIST | OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR,
        },
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

enum FormFieldRuntime {
    TextInput {
        field_id: String,
        edit_control_id: i32,
        warning_control_id: Option<i32>,
        validation: FormTextValidation,
        live_warning: Option<FormFileExistsWarning>,
    },
    CheckBox {
        field_id: String,
        control_id: i32,
    },
}

struct FormDialogData {
    context_tag: String,
    fields: Vec<FormFieldRuntime>,
    note_severities: HashMap<i32, MessageSeverity>,
    buttons: FormButtons,
    confirmed: bool,
    field_values: Vec<FormFieldValue>,
}

// Helper to extract the low word from WPARAM.
fn loword_from_wparam(wparam: WPARAM) -> u16 {
    (wparam.0 & 0xFFFF) as u16
}

fn form_background_brush() -> HBRUSH {
    static BRUSH: OnceLock<usize> = OnceLock::new();
    let raw = *BRUSH.get_or_init(|| unsafe { CreateSolidBrush(COLOR_DIALOG_BG).0 as usize });
    HBRUSH(raw as *mut c_void)
}

fn form_validation_is_valid(validation: &FormTextValidation, value: &str) -> bool {
    match validation {
        FormTextValidation::Any => true,
        FormTextValidation::NonEmpty => !value.trim().is_empty(),
        FormTextValidation::PathSegment => is_safe_path_segment(value),
    }
}

fn is_safe_path_segment(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return false;
    }
    if trimmed.contains(['/', '\\', '\0']) {
        return false;
    }
    !std::path::Path::new(trimmed).is_absolute()
}

fn set_dialog_item_text(hdlg: HWND, control_id: i32, text: &str) {
    let h_text = HSTRING::from(text);
    unsafe {
        SetDlgItemTextW(hdlg, control_id, &h_text).unwrap_or_default();
    }
}

fn read_control_text(hwnd: HWND) -> String {
    match window_common::read_edit_control_text(hwnd) {
        Ok(text) => text,
        Err(err) => {
            log::warn!("DialogHandler: failed to read edit control text: {err}");
            String::new()
        }
    }
}

fn update_form_dialog_live_state(hdlg: HWND) {
    let data_ptr = unsafe { GetWindowLongPtrW(hdlg, GWLP_USERDATA) } as *mut FormDialogData;
    if data_ptr.is_null() {
        return;
    }
    let data = unsafe { &mut *data_ptr };
    let mut any_invalid = false;
    for field in &data.fields {
        if let FormFieldRuntime::TextInput {
            edit_control_id,
            warning_control_id,
            validation,
            live_warning,
            ..
        } = field
        {
            let Ok(hwnd_edit) = (unsafe { GetDlgItem(Some(hdlg), *edit_control_id) }) else {
                any_invalid = true;
                continue;
            };
            let current_text = read_control_text(hwnd_edit);
            let valid = form_validation_is_valid(validation, &current_text);
            any_invalid |= !valid;
            if let Some(warning) = live_warning
                && let Some(warning_control_id) = warning_control_id
            {
                let exists = warning.base_dir.join(current_text.trim()).exists();
                set_dialog_item_text(
                    hdlg,
                    *warning_control_id,
                    if exists { &warning.message } else { "" },
                );
            }
        }
    }
    let enabled = data.buttons.confirm_enabled && !any_invalid;
    if let Ok(hwnd_ok) = unsafe { GetDlgItem(Some(hdlg), ID_DIALOG_FORM_OK) } {
        unsafe {
            let _ = EnableWindow(hwnd_ok, enabled);
        }
    }
}

fn collect_form_field_values(hdlg: HWND, data: &mut FormDialogData) -> Vec<FormFieldValue> {
    let mut values = Vec::new();
    for field in &data.fields {
        match field {
            FormFieldRuntime::TextInput {
                field_id,
                edit_control_id,
                ..
            } => {
                if let Ok(hwnd_edit) = unsafe { GetDlgItem(Some(hdlg), *edit_control_id) } {
                    values.push(FormFieldValue::Text {
                        field_id: field_id.clone(),
                        value: read_control_text(hwnd_edit),
                    });
                }
            }
            FormFieldRuntime::CheckBox {
                field_id,
                control_id,
            } => {
                if let Ok(hwnd_check) = unsafe { GetDlgItem(Some(hdlg), *control_id) } {
                    let checked = unsafe { SendMessageW(hwnd_check, BM_GETCHECK, None, None) }.0
                        as u32
                        == BST_CHECKED.0;
                    values.push(FormFieldValue::CheckBox {
                        field_id: field_id.clone(),
                        checked,
                    });
                }
            }
        }
    }
    values
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
                            match window_common::read_edit_control_text(hwnd_edit_ok) {
                                Ok(text) => dialog_data.input_text = text,
                                Err(err) => {
                                    log::error!(
                                        "DialogHandler: Failed to read input dialog text: {err}"
                                    );
                                    dialog_data.input_text.clear();
                                }
                            }
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
                    if !dialog_data_ptr.is_null()
                        && let Ok(edit_hwnd) = unsafe {
                            GetDlgItem(Some(hdlg), window_common::ID_DIALOG_EXCLUDE_PATTERNS_EDIT)
                        }
                    {
                        let text_len = unsafe { GetWindowTextLengthW(edit_hwnd) } as usize;
                        let mut buffer: Vec<u16> = vec![0; text_len + 1];
                        let written = unsafe { GetWindowTextW(edit_hwnd, buffer.as_mut_slice()) };
                        buffer.truncate(written as usize);
                        let mut result = String::from_utf16_lossy(&buffer);
                        result = result.replace("\r\n", "\n");
                        unsafe {
                            (*dialog_data_ptr).result_text = result;
                            (*dialog_data_ptr).saved = true;
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

fn form_dialog_control_id(base: i32, index: usize) -> i32 {
    base + index as i32
}

fn form_dialog_content_height(form: &FormDialogDescriptor) -> i32 {
    let mut height = 14;
    height += (form.rows.len() as i32) * 18;
    for field in &form.fields {
        match field {
            FormField::TextInput { live_warning, .. } => {
                height += 18;
                height += 24;
                if live_warning.is_some() {
                    height += 16;
                }
            }
            FormField::CheckBox { .. } => {
                height += 18;
            }
        }
    }
    height + 36
}

fn build_form_dialog_template(
    template_bytes: &mut Vec<u8>,
    form: &FormDialogDescriptor,
) -> PlatformResult<()> {
    let mut cdit: u16 = 0;
    cdit = cdit.saturating_add(form.rows.len() as u16);
    for field in &form.fields {
        cdit = cdit.saturating_add(match field {
            FormField::TextInput { live_warning, .. } => {
                2 + if live_warning.is_some() { 1 } else { 0 }
            }
            FormField::CheckBox { .. } => 1,
        });
    }
    cdit = cdit.saturating_add(2);

    let dlg_template = DLGTEMPLATE {
        style: DS_CENTER as u32
            | DS_MODALFRAME as u32
            | DS_SETFONT as u32
            | WS_CAPTION.0
            | WS_SYSMENU.0
            | WS_POPUP.0,
        dwExtendedStyle: 0,
        cdit,
        x: 0,
        y: 0,
        cx: 420,
        cy: form_dialog_content_height(form) as i16,
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(dlg_template) as *const [u8; size_of::<DLGTEMPLATE>()])
    });
    push_word(template_bytes, 0);
    push_word(template_bytes, 0);
    push_str_utf16(template_bytes, &form.title);
    push_word(template_bytes, 8);
    push_str_utf16(template_bytes, "MS Shell Dlg");

    let mut y = 10;
    for (index, row) in form.rows.iter().enumerate() {
        align_to_dword(template_bytes);
        let row_id = form_dialog_control_id(ID_DIALOG_FORM_FIRST_ROW, index);
        let row_text = match row {
            FormRow::ReadOnlyText { label, value } => {
                if label.is_empty() {
                    value.clone()
                } else {
                    format!("{label}: {value}")
                }
            }
            FormRow::Note { text, .. } => text.clone(),
        };
        let row_item = DLGITEMTEMPLATE {
            style: WS_CHILD.0 | WS_VISIBLE.0 | window_common::SS_LEFT.0,
            id: row_id as u16,
            x: 10,
            y,
            cx: 390,
            cy: 14,
            ..Default::default()
        };
        template_bytes.extend_from_slice(unsafe {
            &*(std::ptr::addr_of!(row_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
        });
        push_str_utf16(template_bytes, "Static");
        push_str_utf16(template_bytes, &row_text);
        push_word(template_bytes, 0);
        y += 18;
    }

    for (index, field) in form.fields.iter().enumerate() {
        match field {
            FormField::TextInput {
                label,
                value: _,
                live_warning,
                ..
            } => {
                align_to_dword(template_bytes);
                let label_id = form_dialog_control_id(ID_DIALOG_FORM_FIRST_FIELD_LABEL, index);
                let label_item = DLGITEMTEMPLATE {
                    style: WS_CHILD.0 | WS_VISIBLE.0 | window_common::SS_LEFT.0,
                    id: label_id as u16,
                    x: 10,
                    y,
                    cx: 390,
                    cy: 14,
                    ..Default::default()
                };
                template_bytes.extend_from_slice(unsafe {
                    &*(std::ptr::addr_of!(label_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
                });
                push_str_utf16(template_bytes, "Static");
                push_str_utf16(template_bytes, label);
                push_word(template_bytes, 0);
                y += 16;

                align_to_dword(template_bytes);
                let edit_id = form_dialog_control_id(ID_DIALOG_FORM_FIRST_FIELD_EDIT, index);
                let edit_item = DLGITEMTEMPLATE {
                    style: WS_CHILD.0
                        | WS_VISIBLE.0
                        | WS_BORDER.0
                        | WS_TABSTOP.0
                        | ES_AUTOHSCROLL as u32,
                    id: edit_id as u16,
                    x: 10,
                    y,
                    cx: 390,
                    cy: 14,
                    ..Default::default()
                };
                template_bytes.extend_from_slice(unsafe {
                    &*(std::ptr::addr_of!(edit_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
                });
                push_str_utf16(template_bytes, "Edit");
                push_word(template_bytes, 0);
                push_word(template_bytes, 0);
                y += 18;

                if live_warning.is_some() {
                    align_to_dword(template_bytes);
                    let warning_id = form_dialog_control_id(ID_DIALOG_FORM_FIRST_WARNING, index);
                    let warning_item = DLGITEMTEMPLATE {
                        style: WS_CHILD.0 | WS_VISIBLE.0 | window_common::SS_LEFT.0,
                        id: warning_id as u16,
                        x: 10,
                        y,
                        cx: 390,
                        cy: 14,
                        ..Default::default()
                    };
                    template_bytes.extend_from_slice(unsafe {
                        &*(std::ptr::addr_of!(warning_item)
                            as *const [u8; size_of::<DLGITEMTEMPLATE>()])
                    });
                    push_str_utf16(template_bytes, "Static");
                    push_word(template_bytes, 0);
                    push_word(template_bytes, 0);
                    y += 16;
                }
            }
            FormField::CheckBox { label, .. } => {
                align_to_dword(template_bytes);
                let checkbox_id =
                    form_dialog_control_id(ID_DIALOG_FORM_FIRST_FIELD_CHECKBOX, index);
                let checkbox_item = DLGITEMTEMPLATE {
                    style: WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_AUTOCHECKBOX as u32,
                    id: checkbox_id as u16,
                    x: 10,
                    y,
                    cx: 390,
                    cy: 16,
                    ..Default::default()
                };
                template_bytes.extend_from_slice(unsafe {
                    &*(std::ptr::addr_of!(checkbox_item)
                        as *const [u8; size_of::<DLGITEMTEMPLATE>()])
                });
                push_str_utf16(template_bytes, "Button");
                push_str_utf16(template_bytes, label);
                push_word(template_bytes, 0);
                y += 18;
            }
        }
    }

    align_to_dword(template_bytes);
    let ok_button_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | BS_DEFPUSHBUTTON as u32,
        id: ID_DIALOG_FORM_OK as u16,
        x: 240,
        y,
        cx: 70,
        cy: 16,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(ok_button_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Button");
    push_str_utf16(template_bytes, &form.buttons.confirm_label);
    push_word(template_bytes, 0);

    align_to_dword(template_bytes);
    let cancel_button_item = DLGITEMTEMPLATE {
        style: WS_CHILD.0 | WS_VISIBLE.0 | BS_PUSHBUTTON as u32,
        id: ID_DIALOG_FORM_CANCEL as u16,
        x: 320,
        y,
        cx: 80,
        cy: 16,
        ..Default::default()
    };
    template_bytes.extend_from_slice(unsafe {
        &*(std::ptr::addr_of!(cancel_button_item) as *const [u8; size_of::<DLGITEMTEMPLATE>()])
    });
    push_str_utf16(template_bytes, "Button");
    push_str_utf16(template_bytes, &form.buttons.cancel_label);
    push_word(template_bytes, 0);

    Ok(())
}

unsafe extern "system" fn form_dialog_proc(
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
            let dialog_data = unsafe { &*(lparam.0 as *const FormDialogData) };
            window_common::try_enable_dark_mode(hdlg);

            for field in &dialog_data.fields {
                match field {
                    FormFieldRuntime::TextInput {
                        field_id: runtime_field_id,
                        edit_control_id,
                        warning_control_id,
                        live_warning,
                        validation: _,
                        ..
                    } => {
                        if let Ok(hwnd_edit) = unsafe { GetDlgItem(Some(hdlg), *edit_control_id) } {
                            let initial_text = dialog_data
                                .field_values
                                .iter()
                                .find_map(|value| match value {
                                    FormFieldValue::Text { field_id, value }
                                        if field_id == runtime_field_id =>
                                    {
                                        Some(value.clone())
                                    }
                                    _ => None,
                                })
                                .unwrap_or_default();
                            if !initial_text.is_empty() {
                                set_dialog_item_text(hdlg, *edit_control_id, &initial_text);
                            }
                            window_common::try_enable_dark_mode(hwnd_edit);
                        }
                        if let Some(warning_control_id) = warning_control_id
                            && let Ok(hwnd_warning) =
                                unsafe { GetDlgItem(Some(hdlg), *warning_control_id) }
                        {
                            window_common::try_enable_dark_mode(hwnd_warning);
                            if let Some(warning) = live_warning {
                                set_dialog_item_text(hdlg, *warning_control_id, &warning.message);
                            }
                        }
                    }
                    FormFieldRuntime::CheckBox {
                        field_id: runtime_field_id,
                        control_id,
                    } => {
                        if let Ok(hwnd_check) = unsafe { GetDlgItem(Some(hdlg), *control_id) } {
                            if let Some(initial_checked) =
                                dialog_data
                                    .field_values
                                    .iter()
                                    .find_map(|value| match value {
                                        FormFieldValue::CheckBox { field_id, checked }
                                            if field_id == runtime_field_id =>
                                        {
                                            Some(*checked)
                                        }
                                        _ => None,
                                    })
                            {
                                unsafe {
                                    SendMessageW(
                                        hwnd_check,
                                        BM_SETCHECK,
                                        Some(WPARAM(if initial_checked { 1 } else { 0 })),
                                        Some(LPARAM(0)),
                                    );
                                }
                            }
                            window_common::apply_button_dark_mode_classic_render(hwnd_check);
                        }
                    }
                }
            }

            if let Ok(hwnd_ok) = unsafe { GetDlgItem(Some(hdlg), ID_DIALOG_FORM_OK) } {
                window_common::apply_button_dark_mode_classic_render(hwnd_ok);
                unsafe {
                    let _ = EnableWindow(hwnd_ok, dialog_data.buttons.confirm_enabled);
                }
            }
            if let Ok(hwnd_cancel) = unsafe { GetDlgItem(Some(hdlg), ID_DIALOG_FORM_CANCEL) } {
                window_common::apply_button_dark_mode_classic_render(hwnd_cancel);
            }

            update_form_dialog_live_state(hdlg);
            TRUE.0 as isize
        }
        WM_COMMAND => {
            let command_id = window_common::loword_from_wparam(wparam);
            let notification_code = window_common::highord_from_wparam(wparam);
            match command_id {
                x if x == ID_DIALOG_FORM_OK => {
                    let dialog_data_ptr =
                        unsafe { GetWindowLongPtrW(hdlg, GWLP_USERDATA) } as *mut FormDialogData;
                    if !dialog_data_ptr.is_null() {
                        let dialog_data = unsafe { &mut *dialog_data_ptr };
                        dialog_data.confirmed = true;
                        dialog_data.field_values = collect_form_field_values(hdlg, dialog_data);
                    }
                    unsafe {
                        EndDialog(hdlg, IDOK.0 as isize).unwrap_or_default();
                    }
                    TRUE.0 as isize
                }
                x if x == ID_DIALOG_FORM_CANCEL => {
                    let dialog_data_ptr =
                        unsafe { GetWindowLongPtrW(hdlg, GWLP_USERDATA) } as *mut FormDialogData;
                    if !dialog_data_ptr.is_null() {
                        unsafe { (*dialog_data_ptr).confirmed = false };
                    }
                    unsafe {
                        EndDialog(hdlg, IDCANCEL.0 as isize).unwrap_or_default();
                    }
                    TRUE.0 as isize
                }
                _ => {
                    let dialog_data_ptr =
                        unsafe { GetWindowLongPtrW(hdlg, GWLP_USERDATA) } as *mut FormDialogData;
                    if dialog_data_ptr.is_null() {
                        return FALSE.0 as isize;
                    }
                    let dialog_data = unsafe { &mut *dialog_data_ptr };
                    let mut needs_refresh = false;
                    for field in &dialog_data.fields {
                        match field {
                            FormFieldRuntime::TextInput {
                                edit_control_id, ..
                            } if command_id == *edit_control_id
                                && notification_code == EN_CHANGE as i32 =>
                            {
                                needs_refresh = true;
                            }
                            FormFieldRuntime::CheckBox { control_id, .. }
                                if command_id == *control_id && notification_code == 0 =>
                            {
                                needs_refresh = true;
                            }
                            _ => {}
                        }
                    }
                    if needs_refresh {
                        update_form_dialog_live_state(hdlg);
                        return TRUE.0 as isize;
                    }
                    FALSE.0 as isize
                }
            }
        }
        WM_CTLCOLORDLG | WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
            let hdc = HDC(wparam.0 as *mut c_void);
            let hwnd_control = HWND(lparam.0 as *mut c_void);
            let dialog_data_ptr =
                unsafe { GetWindowLongPtrW(hdlg, GWLP_USERDATA) } as *mut FormDialogData;
            if !dialog_data_ptr.is_null() {
                let dialog_data = unsafe { &mut *dialog_data_ptr };
                unsafe {
                    SetBkColor(hdc, COLOR_DIALOG_BG);
                    SetTextColor(hdc, COLOR_DIALOG_TEXT);
                    SetBkMode(hdc, TRANSPARENT);
                }
                let control_id_raw = unsafe { GetDlgCtrlID(hwnd_control) };
                if let Some(severity) = dialog_data.note_severities.get(&control_id_raw) {
                    unsafe {
                        SetTextColor(
                            hdc,
                            match severity {
                                MessageSeverity::Warning => COLOR_DIALOG_WARNING,
                                MessageSeverity::Error => COLORREF(0x0000_66FF),
                                _ => COLOR_DIALOG_TEXT,
                            },
                        );
                    }
                }
            } else {
                unsafe {
                    SetBkColor(hdc, COLOR_DIALOG_BG);
                    SetTextColor(hdc, COLOR_DIALOG_TEXT);
                    SetBkMode(hdc, TRANSPARENT);
                }
            }
            let _ = hwnd_control;
            form_background_brush().0 as isize
        }
        WM_CLOSE => {
            unsafe {
                EndDialog(hdlg, IDCANCEL.0 as isize).unwrap_or_default();
            }
            TRUE.0 as isize
        }
        _ => FALSE.0 as isize,
    }
}

pub(crate) fn handle_show_form_dialog_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    form: FormDialogDescriptor,
) -> PlatformResult<()> {
    log::debug!(
        "DialogHandler: Showing generic form dialog. Title: '{}' Context: '{}'",
        form.title,
        form.context_tag
    );
    let hwnd_owner = get_hwnd_owner(internal_state, window_id)?;

    let mut fields = Vec::new();
    let mut note_severities = HashMap::new();
    for (index, field) in form.fields.iter().enumerate() {
        match field {
            FormField::TextInput {
                field_id,
                value,
                validation,
                live_warning,
                ..
            } => {
                let edit_control_id =
                    form_dialog_control_id(ID_DIALOG_FORM_FIRST_FIELD_EDIT, index);
                let warning_control_id = live_warning
                    .as_ref()
                    .map(|_| form_dialog_control_id(ID_DIALOG_FORM_FIRST_WARNING, index));
                if let Some(control_id) = warning_control_id {
                    note_severities.insert(control_id, MessageSeverity::Warning);
                }
                fields.push(FormFieldRuntime::TextInput {
                    field_id: field_id.clone(),
                    edit_control_id,
                    warning_control_id,
                    validation: validation.clone(),
                    live_warning: live_warning.clone(),
                });
                let _ = value;
            }
            FormField::CheckBox {
                field_id, checked, ..
            } => {
                fields.push(FormFieldRuntime::CheckBox {
                    field_id: field_id.clone(),
                    control_id: form_dialog_control_id(ID_DIALOG_FORM_FIRST_FIELD_CHECKBOX, index),
                });
                let _ = checked;
            }
        }
    }
    for (index, row) in form.rows.iter().enumerate() {
        if let FormRow::Note { severity, .. } = row {
            note_severities.insert(
                form_dialog_control_id(ID_DIALOG_FORM_FIRST_ROW, index),
                *severity,
            );
        }
    }

    let mut dialog_data = FormDialogData {
        context_tag: form.context_tag.clone(),
        fields,
        note_severities,
        buttons: form.buttons.clone(),
        confirmed: false,
        field_values: form
            .fields
            .iter()
            .map(|field| match field {
                FormField::TextInput {
                    field_id, value, ..
                } => FormFieldValue::Text {
                    field_id: field_id.clone(),
                    value: value.clone(),
                },
                FormField::CheckBox {
                    field_id, checked, ..
                } => FormFieldValue::CheckBox {
                    field_id: field_id.clone(),
                    checked: *checked,
                },
            })
            .collect(),
    };

    let mut template_bytes = Vec::<u8>::new();
    build_form_dialog_template(&mut template_bytes, &form)?;
    let dialog_result = unsafe {
        DialogBoxIndirectParamW(
            Some(internal_state.h_instance()),
            template_bytes.as_ptr() as *const DLGTEMPLATE,
            Some(hwnd_owner),
            Some(form_dialog_proc),
            LPARAM(&mut dialog_data as *mut _ as isize),
        )
    };

    let confirmed = dialog_result == IDOK.0 as isize && dialog_data.confirmed;
    let field_values = if confirmed {
        dialog_data.field_values.clone()
    } else {
        Vec::new()
    };

    internal_state.send_event(AppEvent::FormDialogCompleted {
        window_id,
        context_tag: dialog_data.context_tag,
        confirmed,
        field_values,
    });

    Ok(())
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
    while !vec.len().is_multiple_of(align_of::<u32>()) {
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

fn message_box_icon_flag(severity: MessageSeverity) -> MESSAGEBOX_STYLE {
    match severity {
        MessageSeverity::Error => MB_ICONERROR,
        MessageSeverity::Warning => MB_ICONWARNING,
        _ => MB_ICONINFORMATION,
    }
}

/*
 * Handles the `ShowMessageBox` platform command by displaying a modal Win32 message box.
 * It maps the provided `MessageSeverity` to an icon style, keeping the UX consistent
 * with other status surfaces while ensuring the message appears prominently.
 * [CDU-Dialogs-MessageBoxV1] Message box commands convert severity into the matching Win32 icon flags before showing the modal dialog.
 */
pub(crate) fn handle_show_message_box_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    title: String,
    message: String,
    severity: MessageSeverity,
) -> PlatformResult<()> {
    let hwnd_owner = get_hwnd_owner(internal_state, window_id)?;

    let icon_flag = message_box_icon_flag(severity);

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
 * [CDU-Dialogs-FolderV1] The folder picker command wraps the Win32 IFileOpenDialog with `FOS_PICKFOLDERS`,
 * returning the chosen directory path via AppEvent.
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
            if let Some(dir_path) = &initial_dir
                && let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                    &HSTRING::from(dir_path.as_os_str()),
                    None,
                )
                && let Err(e) = file_dialog.SetDefaultFolder(&item)
            {
                log::error!("DialogHandler: IFileOpenDialog::SetDefaultFolder failed: {e:?}");
            }
            if file_dialog.Show(Some(hwnd_owner)).is_ok()
                && let Ok(shell_item) = file_dialog.GetResult()
                && let Ok(pwstr_path) = shell_item.GetDisplayName(SIGDN_FILESYSPATH)
            {
                let path_string = pwstr_path.to_string().unwrap_or_default();
                CoTaskMemFree(Some(pwstr_path.as_ptr() as *const c_void));
                if !path_string.is_empty() {
                    path_result = Some(PathBuf::from(path_string));
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

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING,
    };

    #[test]
    // [CDU-Dialogs-MessageBoxV1] Severity levels map to the expected Win32 icon flags.
    fn icon_flag_tracks_severity() {
        assert_eq!(
            message_box_icon_flag(MessageSeverity::Information),
            MB_ICONINFORMATION
        );
        assert_eq!(
            message_box_icon_flag(MessageSeverity::Warning),
            MB_ICONWARNING
        );
        assert_eq!(message_box_icon_flag(MessageSeverity::Error), MB_ICONERROR);
    }
}
