use crate::{
    command_executor,
    controls::{
        button_handler, combobox_handler, dialog_handler, label_handler, menu_handler,
        panel_handler, progress_handler, radiobutton_handler, richedit_handler,
        splitter_handler, styling_handler, treeview_handler,
    },
    error::{PlatformError, Result as PlatformResult},
    styling::{ControlStyle, FontWeight, ParsedControlStyle, StyleId},
    types::{
        AppEvent, ControlId, PlatformCommand, PlatformEventHandler, UiStateProvider, WindowConfig,
        WindowId,
    },
    window_common,
};

// [CDU-Tech-WindowsRsV1] All Win32 API interactions below flow through the `windows` crate bindings.
use windows::{
    Win32::{
        Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, WPARAM},
        Graphics::Gdi::{
            CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_QUALITY,
            FF_DONTCARE, FW_BOLD, FW_NORMAL, GetDC, GetDeviceCaps, HBRUSH, HFONT, InvalidateRect,
            LOGPIXELSY, OUT_DEFAULT_PRECIS, ReleaseDC,
        },
        System::Com::{CoInitializeEx, CoUninitialize},
        System::LibraryLoader::{GetModuleHandleW, LoadLibraryW},
        System::WindowsProgramming::MulDiv,
        UI::Controls::{
            ICC_PROGRESS_CLASS, ICC_TREEVIEW_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
            PBM_SETBARCOLOR, PBM_SETBKCOLOR, SetWindowTheme, TVM_SETBKCOLOR, TVM_SETTEXTCOLOR,
        },
        UI::WindowsAndMessaging::*,
    },
    core::{HSTRING, PCWSTR, w},
};

use std::collections::HashMap;
use std::sync::{
    Arc, Mutex, Once, RwLock, Weak,
    atomic::{AtomicUsize, Ordering},
};

static LOAD_RICHEDIT_DLL_ONCE: Once = Once::new();

// Type alias for the complex UI state provider type to reduce type complexity.
type UiStateProviderHolder = Mutex<Option<Weak<Mutex<dyn UiStateProvider>>>>;

/*
 * Internal state for the Win32 platform layer.
 *
 * This struct holds all necessary Win32 handles and mappings required to manage
 * the application's lifecycle and UI elements. It is managed by `PlatformInterface`
 * and accessed by the `WndProc` and command handlers.
 * It is passed as user data to some functions.
 * [CDU-Tech-ThreadSafetyV1] Shared state is wrapped in Arc/RwLock/Mutex primitives so UI commands/events stay data-race free.
 */
pub(crate) struct Win32ApiInternalState {
    h_instance: HINSTANCE,
    next_window_id_counter: AtomicUsize, // For generating unique WindowIds
    // Central registry for all active windows, mapping WindowId to its native state.
    active_windows: RwLock<HashMap<WindowId, window_common::NativeWindowData>>,
    application_event_handler: Mutex<Option<Weak<Mutex<dyn PlatformEventHandler>>>>,
    ui_state_provider: UiStateProviderHolder,
    // Stores processed, native-ready style definitions, keyed by a semantic ID.
    defined_styles: RwLock<HashMap<StyleId, Arc<ParsedControlStyle>>>,
    // The application name, used for window class registration.
    app_name_for_class: String,
    is_quitting: AtomicUsize, // 0 = false, 1 = true
}

// SAFETY: All fields are Send + Sync or wrapped in thread-safe containers, and trait objects are required to be Send + Sync.
unsafe impl Send for Win32ApiInternalState {}
unsafe impl Sync for Win32ApiInternalState {}

impl Win32ApiInternalState {
    /*
     * Generates a new unique `WindowId`.
     */
    pub(crate) fn generate_unique_window_id(&self) -> WindowId {
        WindowId(self.next_window_id_counter.fetch_add(1, Ordering::Relaxed))
    }

    /*
     * Creates and registers a new `NativeWindowData` entry for an upcoming window.
     * By inserting the entry before native creation, WM_NCCREATE handlers can rely
     * on the data being present. Lock poisoning is surfaced as a platform error.
     */
    pub(crate) fn prepare_new_window(&self) -> PlatformResult<WindowId> {
        let window_id = self.generate_unique_window_id();
        let mut windows_map = self.active_windows.write().map_err(|e| {
            log::error!(
                "Win32ApiInternalState: Failed to lock active_windows for preliminary insert of WinID {window_id:?}: {e:?}"
            );
            PlatformError::OperationFailed(
                "Failed to lock active_windows for new window preparation".into(),
            )
        })?;

        if let Some(previous_entry) =
            windows_map.insert(window_id, window_common::NativeWindowData::new(window_id))
        {
            log::error!(
                "Win32ApiInternalState: Replaced existing NativeWindowData while preparing WinID {window_id:?}. Old data will be dropped."
            );
            drop(previous_entry);
        }

        log::debug!(
            "Win32ApiInternalState: Prepared preliminary NativeWindowData for WinID {window_id:?}."
        );
        Ok(window_id)
    }

    /*
     * Retrieves the application's instance handle.
     * Control and window creation functions use this value when calling Win32 APIs.
     */
    pub(crate) fn h_instance(&self) -> HINSTANCE {
        self.h_instance
    }

    #[cfg(test)]
    pub(crate) fn active_windows(
        &self,
    ) -> &RwLock<HashMap<WindowId, window_common::NativeWindowData>> {
        &self.active_windows
    }

    pub(crate) fn ui_state_provider(&self) -> &UiStateProviderHolder {
        &self.ui_state_provider
    }

    pub(crate) fn app_name_for_class(&self) -> &str {
        &self.app_name_for_class
    }

    /*
     * Creates a new instance of `Win32ApiInternalState`.
     * Initializes COM, common controls, and retrieves the application instance handle.
     */
    pub(crate) fn new(app_name_for_class: String) -> PlatformResult<Arc<Self>> {
        unsafe {
            let hr = CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            if hr.is_err()
                && hr != windows::Win32::Foundation::S_FALSE
                && hr != windows::Win32::Foundation::RPC_E_CHANGED_MODE
            {
                return Err(PlatformError::InitializationFailed(format!(
                    "CoInitializeEx failed: {hr:?}"
                )));
            }

            let icex = INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_TREEVIEW_CLASSES | ICC_PROGRESS_CLASS,
            };
            if !InitCommonControlsEx(&icex).as_bool() {
                log::error!(
                    "Warning: InitCommonControlsEx for TreeView failed. Error: {:?}",
                    GetLastError()
                );
            }

            LOAD_RICHEDIT_DLL_ONCE.call_once(|| {
                if let Err(err) = LoadLibraryW(w!("Msftedit.dll")) {
                    log::warn!("Failed to load Msftedit.dll for RichEdit support: {err:?}");
                }
            });

            let h_instance = HINSTANCE(GetModuleHandleW(PCWSTR::null())?.0);
            Ok(Arc::new(Self {
                h_instance,
                next_window_id_counter: AtomicUsize::new(1),
                active_windows: RwLock::new(HashMap::new()),
                application_event_handler: Mutex::new(None),
                ui_state_provider: Mutex::new(None),
                defined_styles: RwLock::new(HashMap::new()),
                app_name_for_class,
                is_quitting: AtomicUsize::new(0),
            }))
        }
    }

    // Sends an AppEvent to the registered application event handler.
    // This centralizes the logic for locking, upgrading the weak reference,
    // and calling the handler.
    pub(crate) fn send_event(self: &Arc<Self>, event: AppEvent) {
        let event_handler_opt = self
            .application_event_handler
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|weak_handler| weak_handler.upgrade());

        if let Some(handler_arc) = event_handler_opt {
            if let Ok(mut handler_guard) = handler_arc.lock() {
                handler_guard.handle_event(event);
            } else {
                log::error!("Platform: Failed to lock event handler to send event.");
            }
        } else {
            log::warn!("Platform: Event handler not available to send event.");
        }
    }

    /*
     * Removes the data for a given window ID from the active windows map.
     * This is a map-level operation that acquires a write lock, removes the
     * entry, and then lets `NativeWindowData`'s Drop implementation clean up
     * any GDI resources it owns.
     */
    pub(crate) fn remove_window_data(&self, window_id: WindowId) {
        let mut windows_map_guard = match self.active_windows.write() {
            Ok(guard) => guard,
            Err(e) => {
                log::error!("Failed to acquire write lock to remove WinID {window_id:?}: {e:?}");
                return; // Exit if the lock is poisoned.
            }
        };

        if windows_map_guard.remove(&window_id).is_some() {
            log::debug!(
                "Removed data for WindowId {window_id:?}; NativeWindowData drop will handle cleanup."
            );
        } else {
            log::warn!("Attempted to remove non-existent WindowId {window_id:?}.");
        }
    }

    /*
     * Associates the native HWND with a prepared window entry. This keeps the HWND
     * update logic centralized and leverages the existing write helper for safety.
     */
    pub(crate) fn attach_hwnd(&self, window_id: WindowId, hwnd: HWND) -> PlatformResult<()> {
        self.with_window_data_write(window_id, |window_data| {
            window_data.set_hwnd(hwnd);
            log::debug!("Win32ApiInternalState: Attached HWND {hwnd:?} to WinID {window_id:?}.");
            Ok(())
        })
    }

    /*
     * A specialized helper for mutating the TreeView's internal state.
     * This function safely takes the TreeView state out of the `NativeWindowData`,
     * executes the provided closure on it *without* holding the main window map lock,
     * and then puts the (potentially modified) state back. This is critical for
     * long-running operations like populating the tree, as it prevents deadlocks
     * and avoids blocking other UI commands.
     */
    pub(crate) fn with_treeview_state_mut<F>(
        self: &Arc<Self>,
        window_id: WindowId,
        control_id: ControlId,
        f: F,
    ) -> PlatformResult<()>
    where
        F: FnOnce(HWND, &mut treeview_handler::TreeViewInternalState) -> PlatformResult<()>,
    {
        // Phase 1: Lock, get HWND, and take the treeview state out.
        let (hwnd_treeview, mut taken_tv_state) =
        self.with_window_data_write(window_id, |window_data| {
            let hwnd = window_data.get_control_hwnd(control_id).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "TreeView HWND not found for control ID {}",
                    control_id.raw()
                ))
            })?;

            // Take the state. If it doesn't exist, create a new one for the operation.
            let state = window_data.take_treeview_state().unwrap_or_else(|| {
                log::warn!(
                    "TreeView state was None for WinID {window_id:?}/ControlID {}. Creating new for operation.",
                    control_id.raw()
                );
                treeview_handler::TreeViewInternalState::new()
            });

            Ok((hwnd, state))
        })?;

        // Phase 2: Perform the long-running operation on the state without holding the map lock.
        let result = f(hwnd_treeview, &mut taken_tv_state);

        // Phase 3: Lock again to put the state back, regardless of whether the operation succeeded.
        // This ensures the state is never lost.
        if let Err(e) = self.with_window_data_write(window_id, |window_data| {
            window_data.set_treeview_state(Some(taken_tv_state));
            Ok(())
        }) {
            log::error!(
                "CRITICAL: Failed to put back TreeView state for WinID {window_id:?}. State may be lost. Error: {e:?}"
            );
        }

        // Return the original result from the operation.
        result
    }

    // Provides safe, read-only access to a specific window's data.
    // Handles locking and error checking.
    pub(crate) fn with_window_data_read<F, R>(&self, window_id: WindowId, f: F) -> PlatformResult<R>
    where
        F: FnOnce(&window_common::NativeWindowData) -> PlatformResult<R>,
    {
        let windows_map = self.active_windows.read().map_err(|e| {
            log::error!("Failed to acquire read lock on active_windows: {e:?}");
            PlatformError::OperationFailed("RwLock poisoned".into())
        })?;

        let window_data = windows_map.get(&window_id).ok_or_else(|| {
            log::warn!("Attempted to access non-existent WindowId {window_id:?}");
            PlatformError::InvalidHandle(format!("WindowId {window_id:?} not found"))
        })?;

        f(window_data)
    }

    // Provides safe, writeable access to a specific window's data.
    // Handles locking and error checking.
    pub(crate) fn with_window_data_write<F, R>(
        &self,
        window_id: WindowId,
        f: F,
    ) -> PlatformResult<R>
    where
        F: FnOnce(&mut window_common::NativeWindowData) -> PlatformResult<R>,
    {
        let mut windows_map = self.active_windows.write().map_err(|e| {
            log::error!("Failed to acquire write lock on active_windows: {e:?}");
            PlatformError::OperationFailed("RwLock poisoned".into())
        })?;

        let window_data = windows_map.get_mut(&window_id).ok_or_else(|| {
            log::warn!("Attempted to access non-existent WindowId {window_id:?}");
            PlatformError::InvalidHandle(format!("WindowId {window_id:?} not found"))
        })?;

        f(window_data)
    }

    /*
     * Returns `true` if no active windows remain. This pure check can be unit
     * tested without side effects and drives the quit logic when a window
     * closes.
     */
    fn should_quit_on_last_window_close(&self) -> bool {
        self.active_windows.read().map_or_else(
            |poisoned_err| {
                log::error!(
                    "Win32ApiInternalState: Poisoned RwLock on active_windows during quit check: {poisoned_err:?}"
                );
                false
            },
            |guard| guard.is_empty(),
        )
    }

    /*
     * Called after a window is removed from the `active_windows` map. If this
     * was the last window, posts `WM_QUIT` so the message loop exits. The
     * `is_quitting` flag ensures we honor a prior quit request once all windows
     * have closed.
     * [CDU-AppQuitV1] Posting `WM_QUIT` here gives the app logic a declarative `QuitApplication` command that stops the message pump cleanly.
     */
    pub(crate) fn check_if_should_quit_after_window_close(&self) {
        if self.should_quit_on_last_window_close() {
            log::debug!(
                "Platform: Last active window closed (or was already closed and quit signaled). Posting WM_QUIT."
            );
            unsafe { PostQuitMessage(0) };
        }
    }

    /*
     * Executes a single platform command.
     * Delegates to specific handlers in `command_executor`, `dialog_handler`,
     * or now directly to control handlers (e.g., `treeview_handler`).
     */
    fn execute_platform_command(self: &Arc<Self>, command: PlatformCommand) -> PlatformResult<()> {
        log::trace!("Platform: Executing command: {command:?}");
        match command {
            PlatformCommand::SetWindowTitle { window_id, title } => {
                command_executor::execute_set_window_title(self, window_id, &title)
            }
            PlatformCommand::ShowWindow { window_id } => {
                command_executor::execute_show_window(self, window_id, true)
            }
            PlatformCommand::CloseWindow { window_id } => {
                command_executor::execute_close_window(self, window_id)
            }
            PlatformCommand::PopulateTreeView {
                window_id,
                control_id,
                items,
            } => command_executor::execute_populate_treeview(self, window_id, control_id, items),
            PlatformCommand::UpdateTreeItemVisualState {
                window_id,
                control_id,
                item_id,
                new_state,
            } => command_executor::execute_update_tree_item_visual_state(
                self, window_id, control_id, item_id, new_state,
            ),
            PlatformCommand::UpdateTreeItemText {
                window_id,
                control_id,
                item_id,
                text,
            } => command_executor::execute_update_tree_item_text(
                self, window_id, control_id, item_id, text,
            ),
            PlatformCommand::ShowSaveFileDialog {
                window_id,
                title,
                default_filename,
                filter_spec,
                initial_dir,
            } => dialog_handler::handle_show_save_file_dialog_command(
                self,
                window_id,
                title,
                default_filename,
                filter_spec,
                initial_dir,
            ),
            PlatformCommand::ShowOpenFileDialog {
                window_id,
                title,
                filter_spec,
                initial_dir,
            } => dialog_handler::handle_show_open_file_dialog_command(
                self,
                window_id,
                title,
                filter_spec,
                initial_dir,
            ),
            PlatformCommand::ShowProfileSelectionDialog {
                window_id,
                available_profiles,
                title,
                prompt,
            } => dialog_handler::handle_show_profile_selection_dialog_command(
                self,
                window_id,
                available_profiles,
                title,
                prompt,
            ),
            PlatformCommand::ShowInputDialog {
                window_id,
                title,
                prompt,
                default_text,
                context_tag,
            } => dialog_handler::handle_show_input_dialog_command(
                self,
                window_id,
                title,
                prompt,
                default_text,
                context_tag,
            ),
            PlatformCommand::ShowExcludePatternsDialog {
                window_id,
                title,
                patterns,
            } => dialog_handler::handle_show_exclude_patterns_dialog_command(
                self, window_id, title, patterns,
            ),
            PlatformCommand::ShowMessageBox {
                window_id,
                title,
                message,
                severity,
            } => dialog_handler::handle_show_message_box_command(
                self, window_id, title, message, severity,
            ),
            PlatformCommand::ShowFolderPickerDialog {
                window_id,
                title,
                initial_dir,
            } => dialog_handler::handle_show_folder_picker_dialog_command(
                self,
                window_id,
                title,
                initial_dir,
            ),
            PlatformCommand::SetControlEnabled {
                window_id,
                control_id,
                enabled,
            } => {
                command_executor::execute_set_control_enabled(self, window_id, control_id, enabled)
            }
            PlatformCommand::QuitApplication => {
                // Set an atomic flag indicating that a quit has been requested.
                // This helps the message loop make a final decision if GetMessageW itself errors out
                // but there are no windows left and a quit was intended.
                self.is_quitting.store(1, Ordering::Relaxed);
                command_executor::execute_quit_application()
            }
            PlatformCommand::CreateMainMenu {
                window_id,
                menu_items,
            } => menu_handler::handle_create_main_menu_command(self, window_id, menu_items),
            PlatformCommand::CreateButton {
                window_id,
                parent_control_id,
                control_id,
                text,
            } => button_handler::handle_create_button_command(
                self,
                window_id,
                parent_control_id,
                control_id,
                text,
            ),
            PlatformCommand::CreateTreeView {
                window_id,
                parent_control_id,
                control_id,
            } => treeview_handler::handle_create_treeview_command(
                self,
                window_id,
                parent_control_id,
                control_id,
            ),
            PlatformCommand::SignalMainWindowUISetupComplete { window_id } => {
                command_executor::execute_signal_main_window_ui_setup_complete(self, window_id)
            }
            PlatformCommand::DefineLayout { window_id, rules } => {
                command_executor::execute_define_layout(self, window_id, rules)
            }
            PlatformCommand::CreatePanel {
                window_id,
                parent_control_id,
                control_id: panel_id,
            } => panel_handler::handle_create_panel_command(
                self,
                window_id,
                parent_control_id,
                panel_id,
            ),
            PlatformCommand::CreateLabel {
                window_id,
                parent_control_id,
                control_id: label_id,
                initial_text,
                class,
            } => label_handler::handle_create_label_command(
                self,
                window_id,
                parent_control_id,
                label_id,
                initial_text,
                class,
            ),
            PlatformCommand::UpdateLabelText {
                window_id,
                control_id: label_id,
                text,
                severity,
            } => label_handler::handle_update_label_text_command(
                self, window_id, label_id, text, severity,
            ),
            PlatformCommand::RedrawTreeItem {
                window_id,
                control_id,
                item_id,
            } => treeview_handler::handle_redraw_tree_item_command(
                self, window_id, control_id, item_id,
            ),
            PlatformCommand::ExpandVisibleTreeItems {
                window_id,
                control_id,
            } => command_executor::execute_expand_visible_tree_items(self, window_id, control_id),
            PlatformCommand::ExpandAllTreeItems {
                window_id,
                control_id,
            } => command_executor::execute_expand_all_tree_items(self, window_id, control_id),
            PlatformCommand::CreateInput {
                window_id,
                parent_control_id,
                control_id,
                initial_text,
                read_only,
                multiline,
                vertical_scroll,
            } => command_executor::execute_create_input(
                self,
                window_id,
                parent_control_id,
                control_id,
                command_executor::InputCreationOptions {
                    initial_text,
                    read_only,
                    multiline,
                    vertical_scroll,
                },
            ),
            PlatformCommand::CreateRichEdit {
                window_id,
                parent_control_id,
                control_id,
            } => command_executor::execute_create_rich_edit(
                self,
                window_id,
                parent_control_id,
                control_id,
            ),
            PlatformCommand::CreateProgressBar {
                window_id,
                parent_control_id,
                control_id,
            } => progress_handler::handle_create_progress_bar_command(
                self,
                window_id,
                parent_control_id,
                control_id,
            ),
            PlatformCommand::CreateSplitter {
                window_id,
                parent_control_id,
                control_id,
                orientation,
            } => splitter_handler::handle_create_splitter_command(
                self,
                window_id,
                parent_control_id,
                control_id,
                orientation,
            ),
            PlatformCommand::CreateComboBox {
                window_id,
                parent_control_id,
                control_id,
            } => combobox_handler::handle_create_combobox_command(
                self,
                window_id,
                parent_control_id,
                control_id,
            ),
            PlatformCommand::SetComboBoxItems {
                window_id,
                control_id,
                items,
            } => combobox_handler::handle_set_combobox_items(self, window_id, control_id, items),
            PlatformCommand::SetComboBoxSelection {
                window_id,
                control_id,
                selected_index,
            } => combobox_handler::handle_set_combobox_selection(
                self,
                window_id,
                control_id,
                selected_index,
            ),
            PlatformCommand::CreateRadioButton {
                window_id,
                parent_control_id,
                control_id,
                text,
                group_start,
            } => radiobutton_handler::handle_create_radiobutton_command(
                self,
                window_id,
                parent_control_id,
                control_id,
                text,
                group_start,
            ),
            PlatformCommand::SetProgressBarRange {
                window_id,
                control_id,
                min,
                max,
            } => progress_handler::handle_set_progress_bar_range(
                self, window_id, control_id, min, max,
            ),
            PlatformCommand::SetProgressBarPosition {
                window_id,
                control_id,
                position,
            } => progress_handler::handle_set_progress_bar_position(
                self, window_id, control_id, position,
            ),
            PlatformCommand::SetControlText {
                window_id,
                control_id,
                text,
            } => command_executor::execute_set_control_text(self, window_id, control_id, text),
            PlatformCommand::SetInputText {
                window_id,
                control_id,
                text,
            } => command_executor::execute_set_input_text(self, window_id, control_id, text),
            PlatformCommand::SetViewerContent {
                window_id,
                control_id,
                text,
            } => command_executor::execute_set_viewer_content(self, window_id, control_id, text),
            PlatformCommand::SetRichEditContent {
                window_id,
                control_id,
                rtf_text,
            } => command_executor::execute_set_rich_edit_content(
                self, window_id, control_id, rtf_text,
            ),
            PlatformCommand::SetScrollPosition {
                window_id,
                control_id,
                vertical_pos,
                horizontal_pos,
            } => command_executor::execute_set_scroll_position(
                self,
                window_id,
                control_id,
                vertical_pos,
                horizontal_pos,
            ),
            PlatformCommand::SetTreeViewSelection {
                window_id,
                control_id,
                item_id,
            } => command_executor::execute_set_treeview_selection(
                self, window_id, control_id, item_id,
            ),
            PlatformCommand::DefineStyle { style_id, style } => self.define_style(style_id, style),
            PlatformCommand::ApplyStyleToControl {
                window_id,
                control_id,
                style_id,
            } => self.execute_apply_style_to_control(window_id, control_id, style_id),
        }
    }

    /*
     * Translates a platform-agnostic `ControlStyle` into native Win32 resources and stores
     * the resulting `ParsedControlStyle` inside the shared registry. All GDI objects are
     * wrapped by `ParsedControlStyle`, ensuring deterministic cleanup when they drop.
     *
     * Centralizing the parsing here keeps the style map private while still allowing
     * callers to define styles with a single method call. Early returns prevent partially
     * constructed styles from polluting the registry.
     */
    pub(crate) fn define_style(
        self: &Arc<Self>,
        style_id: StyleId,
        style: ControlStyle,
    ) -> PlatformResult<()> {
        // [CDU-Styling-DefineV1] Style definitions are parsed once and stored so multiple controls can reference them by `StyleId`.
        log::debug!("Win32ApiInternalState: define_style for StyleId::{style_id:?}");

        // --- Parse FontDescription into HFONT ---
        let font_handle: Option<HFONT> = if let Some(font_desc) = style.font.as_ref() {
            let hdc_screen = unsafe { GetDC(None) };
            if hdc_screen.is_invalid() {
                log::error!("Win32ApiInternalState: Could not acquire screen DC for style font.");
                return Err(PlatformError::OperationFailed(
                    "Could not acquire screen DC for font creation".to_string(),
                ));
            }

            let logical_font_height = if let Some(point_size) = font_desc.size {
                -unsafe { MulDiv(point_size, GetDeviceCaps(Some(hdc_screen), LOGPIXELSY), 72) }
            } else {
                0
            };

            unsafe { ReleaseDC(None, hdc_screen) };

            let weight = match font_desc.weight {
                Some(FontWeight::Bold) => FW_BOLD.0 as i32,
                _ => FW_NORMAL.0 as i32,
            };

            let font_name = font_desc.name.as_deref().unwrap_or("MS Shell Dlg 2");
            let font_name_hstring = HSTRING::from(font_name);

            let h_font = unsafe {
                CreateFontW(
                    logical_font_height,
                    0,
                    0,
                    0,
                    weight,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    DEFAULT_QUALITY,
                    FF_DONTCARE.0 as u32,
                    &font_name_hstring,
                )
            };

            if h_font.is_invalid() {
                log::error!(
                    "Win32ApiInternalState: CreateFontW failed while defining StyleId::{style_id:?}: {:?}",
                    unsafe { GetLastError() }
                );
                return Err(PlatformError::OperationFailed(
                    "CreateFontW failed during style definition".to_string(),
                ));
            }
            Some(h_font)
        } else {
            None
        };

        // --- Parse background_color into HBRUSH ---
        let background_brush: Option<HBRUSH> = if let Some(color) = style.background_color.as_ref()
        {
            let color_ref = styling_handler::color_to_colorref(color);
            let h_brush = unsafe { CreateSolidBrush(color_ref) };
            if h_brush.is_invalid() {
                log::error!(
                    "Win32ApiInternalState: CreateSolidBrush failed while defining StyleId::{style_id:?}: {:?}",
                    unsafe { GetLastError() }
                );
                return Err(PlatformError::OperationFailed(
                    "CreateSolidBrush failed during style definition".to_string(),
                ));
            }
            Some(h_brush)
        } else {
            None
        };

        let parsed_style = ParsedControlStyle {
            font_handle,
            text_color: style.text_color,
            background_color: style.background_color,
            background_brush,
        };

        match self.defined_styles.write() {
            Ok(mut styles_map) => {
                styles_map.insert(style_id, Arc::new(parsed_style));
                log::debug!("Successfully stored parsed style for StyleId::{style_id:?}");
            }
            Err(e) => {
                log::error!("Failed to acquire write lock on defined_styles map: {e:?}");
                return Err(PlatformError::OperationFailed(
                    "RwLock poisoned on defined_styles map".to_string(),
                ));
            }
        }
        if style_id == StyleId::MainWindowBackground {
            if let Ok(windows_map) = self.active_windows.read() {
                for window_data in windows_map.values() {
                    let hwnd = window_data.get_hwnd();
                    if !hwnd.is_invalid() {
                        window_common::try_enable_dark_mode(hwnd);
                    }
                }
            } else {
                log::warn!("Failed to acquire read lock to enable dark mode on active windows.");
            }
        }
        Ok(())
    }

    /*
     * Executes the `ApplyStyleToControl` command.
     *
     * This method applies a previously defined style to a specific control. It
     * updates the window's internal mapping, sends a `WM_SETFONT` message if a
     * font is part of the style, and invalidates the control to force a repaint,
     * which will trigger color changes via `WM_CTLCOLOR...` messages.
     */
    fn execute_apply_style_to_control(
        self: &Arc<Self>,
        window_id: WindowId,
        control_id: ControlId,
        style_id: StyleId,
    ) -> PlatformResult<()> {
        log::debug!(
            "Applying style {style_id:?} to ControlID {} in WinID {window_id:?}",
            control_id.raw()
        );

        // Get the control's HWND and store the style association in the window's data.
        let control_hwnd = self.with_window_data_write(window_id, |window_data| {
            window_data.apply_style_to_control(control_id, style_id);
            window_data.get_control_hwnd(control_id).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "Control ID {} not found in WinID {window_id:?}",
                    control_id.raw()
                ))
            })
        })?;

        let parsed_style = self.get_parsed_style(style_id);

        // Check if this is a button and enable owner-draw if style provides colors
        if let Ok(Some(control_kind)) = self.with_window_data_read(window_id, |window_data| {
            Ok(window_data.get_control_kind(control_id))
        }) {
            if control_kind == window_common::ControlKind::Button {
                if let Some(ref style) = parsed_style
                    && style.background_color.is_some()
                    && style.text_color.is_some()
                {
                    unsafe {
                        let current_style =
                            WINDOW_STYLE(GetWindowLongW(control_hwnd, GWL_STYLE) as u32);
                        let new_style = (current_style & !WINDOW_STYLE(BS_TYPEMASK as u32))
                            | WINDOW_STYLE(BS_OWNERDRAW as u32);
                        SetWindowLongW(control_hwnd, GWL_STYLE, new_style.0 as i32);
                        // SWP_FRAMECHANGED forces the window to recalculate its frame
                        _ = SetWindowPos(
                            control_hwnd,
                            None,
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
                        );
                        _ = InvalidateRect(Some(control_hwnd), None, true);
                    }
                }
            }
            // Check if this is a progress bar
            else if control_kind == window_common::ControlKind::ProgressBar {
                unsafe {
                    // Disable visual styles to enable custom colors
                    // Use empty strings, not null pointers
                    let empty = HSTRING::new();
                    _ = SetWindowTheme(control_hwnd, &empty, &empty);

                    // Apply colors
                    if let Some(ref style) = parsed_style {
                        if let Some(bg_color) = &style.background_color {
                            let colorref = styling_handler::color_to_colorref(bg_color);
                            SendMessageW(
                                control_hwnd,
                                PBM_SETBKCOLOR,
                                None,
                                Some(LPARAM(colorref.0 as isize)),
                            );
                        }
                        if let Some(bar_color) = &style.text_color {
                            // Repurpose text_color as bar fill color
                            let colorref = styling_handler::color_to_colorref(bar_color);
                            SendMessageW(
                                control_hwnd,
                                PBM_SETBARCOLOR,
                                None,
                                Some(LPARAM(colorref.0 as isize)),
                            );
                        }
                        _ = InvalidateRect(Some(control_hwnd), None, true);
                    }
                }
            }
            // RichEdit uses dedicated messages for background/text color.
            else if control_kind == window_common::ControlKind::RichEdit
                && let Some(ref style) = parsed_style
            {
                let (background, foreground) = richedit_handler::style_colors_for_rich_edit(style);
                richedit_handler::apply_rich_edit_colors(control_hwnd, background, foreground);
            }
        }

        if let Some(parsed_style) = parsed_style {
            // Apply the font if one is defined in the style.
            if let Some(hfont) = parsed_style.font_handle
                && !hfont.is_invalid()
            {
                unsafe {
                    // SendMessageW is synchronous. The LPARAM(1) tells the control to redraw immediately.
                    SendMessageW(
                        control_hwnd,
                        WM_SETFONT,
                        Some(WPARAM(hfont.0 as usize)),
                        Some(LPARAM(1)),
                    );
                }
            }

            // For TreeView, we need to set colors directly.
            if style_id == StyleId::TreeView {
                if let Some(color) = &parsed_style.background_color {
                    let color_ref = styling_handler::color_to_colorref(color);
                    unsafe {
                        SendMessageW(
                            control_hwnd,
                            TVM_SETBKCOLOR,
                            None,
                            Some(LPARAM(color_ref.0 as isize)),
                        );
                    }
                }
                if let Some(color) = &parsed_style.text_color {
                    let color_ref = styling_handler::color_to_colorref(color);
                    unsafe {
                        SendMessageW(
                            control_hwnd,
                            TVM_SETTEXTCOLOR,
                            None,
                            Some(LPARAM(color_ref.0 as isize)),
                        );
                    }
                }
            }

            // Invalidate the control's rectangle to force a repaint.
            // This is crucial for `WM_CTLCOLOR...` messages to be sent, which will apply
            // the new text and background colors.
            unsafe {
                _ = InvalidateRect(Some(control_hwnd), None, true);
            }
        } else {
            log::warn!("Attempted to apply undefined StyleId::{style_id:?}");
        }

        Ok(())
    }

    /*
     * Retrieves a shared reference to a parsed style definition.
     *
     * This method provides safe, read-only access to the central style map.
     * It returns an `Arc<ParsedControlStyle>`, allowing multiple components to
     * share ownership of the style data (including native GDI handles) without
     * risking double-free errors.
     */
    pub(crate) fn get_parsed_style(&self, style_id: StyleId) -> Option<Arc<ParsedControlStyle>> {
        self.defined_styles
            .read()
            .ok()
            .and_then(|styles_map| styles_map.get(&style_id).cloned())
    }
}

impl Drop for Win32ApiInternalState {
    fn drop(&mut self) {
        log::debug!("Platform: Win32ApiInternalState dropped, calling CoUninitialize.");
        unsafe { CoUninitialize() };
    }
}

/*
 * Provides the main interface for the application to interact with the
 * underlying Win32 platform. It handles window creation, command execution,
 * and running the main event loop.
 */
pub struct PlatformInterface {
    internal_state: Arc<Win32ApiInternalState>,
}

impl PlatformInterface {
    /*
     * Creates a new `PlatformInterface`.
     * Initializes the internal Win32 state and registers the main window class.
     */
    pub fn new(app_name_for_class: String) -> PlatformResult<Self> {
        window_common::init_app_dark_mode();
        let internal_state = Win32ApiInternalState::new(app_name_for_class)?;
        window_common::register_window_class(&internal_state)?;
        log::debug!(
            "Platform: Window class registration attempted during PlatformInterface::new()."
        );
        Ok(PlatformInterface { internal_state })
    }

    /*
     * A `WindowId` is generated and associated with the native window's state.
     * The window is not shown until a `PlatformCommand::ShowWindow` is received.
     * [CDU-WindowCreationV1] Creating a top-level window wires logical metadata to the native HWND before it is ever shown.
     */
    pub fn create_window(&self, config: WindowConfig) -> PlatformResult<WindowId> {
        let window_id = self.internal_state.prepare_new_window()?;

        // Now, create the actual native window.
        let hwnd = match window_common::create_native_window(
            &self.internal_state, // Pass Arc<Win32ApiInternalState>
            window_id,            // Pass the generated WindowId
            config.title,
            config.width,
            config.height,
        ) {
            Ok(h) => h,
            Err(e) => {
                // If native window creation fails, remove the preliminary data.
                self.internal_state.remove_window_data(window_id);
                log::debug!(
                    "Platform: Rolled back prepared NativeWindowData after window creation failure for WindowId {window_id:?}."
                );
                return Err(e);
            }
        };
        log::debug!(
            "Platform: Native window created with HWND {hwnd:?} for WindowId {window_id:?}"
        );

        if let Err(e) = self.internal_state.attach_hwnd(window_id, hwnd) {
            log::error!(
                "Platform: Failed to attach HWND for WindowId {window_id:?}. Cleaning up. Error: {e:?}"
            );
            if !hwnd.is_invalid() {
                unsafe {
                    DestroyWindow(hwnd).ok();
                }
            }
            self.internal_state.remove_window_data(window_id);
            return Err(e);
        }

        Ok(window_id)
    }

    /*
     * Takes the application's event handler and a list of initial commands.
     * Processes initial commands, then enters the message loop, dequeuing and
     * executing commands from the event handler before processing OS messages.
     * Returns when the application quits.
     * [CDU-CmdEventPatternV1] The main loop dequeues `PlatformCommand`s first and then emits `AppEvent`s back to the handler, enforcing the command/event contract.
     */
    pub fn main_event_loop(
        &self,
        event_handler_param: Arc<Mutex<dyn PlatformEventHandler>>,
        ui_state_provider_param: Arc<Mutex<dyn UiStateProvider>>,
        initial_commands_to_execute: Vec<PlatformCommand>,
    ) -> PlatformResult<()> {
        *self
            .internal_state
            .application_event_handler
            .lock()
            .map_err(|e| {
                log::error!("Platform: Failed to lock application_event_handler to set it: {e:?}");
                PlatformError::OperationFailed("Failed to set application event handler".into())
            })? = Some(Arc::downgrade(&event_handler_param));

        *self.internal_state.ui_state_provider.lock().map_err(|e| {
            log::error!("Platform: Failed to lock ui_state_provider to set it: {e:?}");
            PlatformError::OperationFailed("Failed to set ui_state_provider".into())
        })? = Some(Arc::downgrade(&ui_state_provider_param));

        log::debug!(
            "Platform: run() called. Processing {} initial UI commands before event loop.",
            initial_commands_to_execute.len()
        );

        for command in initial_commands_to_execute {
            log::debug!("Platform: Executing initial command: {command:?}");
            if let Err(e) = self.internal_state.execute_platform_command(command) {
                log::error!(
                    "Platform: Error executing initial UI command: {e:?}. Halting initialization."
                );
                return Err(e);
            }
        }
        log::debug!("Platform: Initial UI commands processed successfully.");

        let app_logic_ref_for_loop = event_handler_param;
        unsafe {
            let mut msg = MSG::default();
            loop {
                // Process all pending commands from app logic first
                loop {
                    let command_to_execute: Option<PlatformCommand> = {
                        match app_logic_ref_for_loop.lock() {
                            Ok(mut logic_guard) => logic_guard.try_dequeue_command(),
                            Err(e) => {
                                log::error!(
                                    "Platform: Failed to lock application logic to dequeue command: {e:?}. Skipping command processing for this cycle."
                                );
                                None // Avoid panic, try again next cycle
                            }
                        }
                    };

                    if let Some(command) = command_to_execute {
                        if let Err(e) = self.internal_state.execute_platform_command(command) {
                            log::error!("Platform: Error executing command from queue: {e:?}");
                            // Decide if error is fatal. For now, continue.
                        }
                    } else {
                        break; // No more commands from app logic, proceed to OS messages
                    }
                }

                // Process at most one OS message per cycle. If no messages are queued,
                // sleep briefly so command dequeue stays responsive without user input.
                if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        log::debug!(
                            "Platform: PeekMessageW received WM_QUIT, exiting message loop."
                        );
                        break;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
            }
        }
        // Call on_quit on the event handler
        if let Ok(mut handler_guard) = app_logic_ref_for_loop.lock() {
            handler_guard.on_quit();
        } else {
            log::error!("Platform: Failed to lock application logic for on_quit call.");
        }
        // Clear the event handler reference
        match self.internal_state.application_event_handler.lock() {
            Ok(mut guard) => *guard = None,
            Err(e) => {
                log::error!(
                    "Platform: Failed to lock application_event_handler to clear it (poisoned): {e:?}"
                );
            }
        }
        // Clear the ui state provider reference
        match self.internal_state.ui_state_provider.lock() {
            Ok(mut guard) => *guard = None,
            Err(e) => {
                log::error!(
                    "Platform: Failed to lock ui_state_provider to clear it (poisoned): {e:?}"
                );
            }
        }
        log::debug!("Platform: Message loop exited cleanly.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;
    use crate::types::TreeItemId;
    use crate::window_common::NativeWindowData;
    use windows::Win32::{Foundation::HWND, UI::Controls::HTREEITEM};

    // Helper function to create PathBuf from a slice of u16 (wide char buffer)
    // This is useful when dealing with paths from Win32 API calls.
    pub fn pathbuf_from_buf(buffer: &[u16]) -> PathBuf {
        // Find the first null terminator, or use the whole buffer if none is found.
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        let path_os_string = OsString::from_wide(&buffer[..len]);
        PathBuf::from(path_os_string)
    }

    #[test]
    // [CDU-Dialogs-FileV1][CDU-Dialogs-FolderV1] File/folder dialog buffers round-trip into `PathBuf` using the UTF-16 helper exercised here.
    fn roundtrip_simple() {
        let mut wide: Vec<u16> = "C:\\temp\\file.txt".encode_utf16().collect();
        wide.push(0); // Add null terminator
        let path = pathbuf_from_buf(&wide);
        assert_eq!(path, PathBuf::from(r"C:\\temp\\file.txt"));
    }

    #[test]
    // [CDU-Dialogs-FileV1][CDU-Dialogs-FolderV1] Even when Win32 omits a null terminator, dialog paths still deserialize deterministically.
    fn no_null_terminator() {
        let wide: Vec<u16> = "D:\\data\\incomplete".encode_utf16().collect();
        // No null terminator added
        let path = pathbuf_from_buf(&wide);
        assert_eq!(path, PathBuf::from(r"D:\\data\\incomplete"));
    }

    // Arrange common state for Win32ApiInternalState tests
    fn setup_state() -> (Arc<Win32ApiInternalState>, WindowId, NativeWindowData) {
        let state = Win32ApiInternalState::new("TestState".to_string()).unwrap();
        let window_id = state.generate_unique_window_id();
        let native = NativeWindowData::new(window_id);
        (state, window_id, native)
    }

    #[test]
    // [CDU-WindowCreationV1] Preparing a window reserves a slot in the native map before the HWND exists.
    fn prepare_new_window_registers_entry() {
        // Arrange
        let state = Win32ApiInternalState::new("PrepareTest".to_string()).unwrap();
        // Act
        let window_id = state.prepare_new_window().expect("prepare window");
        // Assert
        let guard = state.active_windows().read().unwrap();
        assert!(guard.contains_key(&window_id));
    }

    #[test]
    // [CDU-WindowCreationV1] Attaching the HWND fulfills the window creation contract by wiring the logical ID to the native handle.
    fn attach_hwnd_updates_native_data() {
        // Arrange
        let state = Win32ApiInternalState::new("AttachTest".to_string()).unwrap();
        let window_id = state.prepare_new_window().expect("prepare window");
        let test_hwnd = HWND(0x1234usize as _);
        // Act
        state
            .attach_hwnd(window_id, test_hwnd)
            .expect("attach hwnd");
        // Assert
        let guard = state.active_windows().read().unwrap();
        let stored = guard.get(&window_id).unwrap();
        assert_eq!(stored.get_hwnd(), test_hwnd);
    }

    #[test]
    // [CDU-Styling-DefineV1] Style definitions are parsed and retained so later commands can reference them by `StyleId`.
    fn define_style_stores_parsed_style() {
        // Arrange
        let state = Win32ApiInternalState::new("StyleTest".to_string()).unwrap();
        let style = ControlStyle::default();
        // Act
        let result = state.define_style(StyleId::DefaultText, style);
        // Assert
        assert!(result.is_ok());
        let parsed = state
            .get_parsed_style(StyleId::DefaultText)
            .expect("style stored");
        assert!(parsed.font_handle.is_none());
        assert!(parsed.background_brush.is_none());
    }

    #[test]
    fn generate_unique_window_id_produces_unique_values() {
        // Arrange
        let state = Win32ApiInternalState::new("UIDTest".to_string()).unwrap();
        // Act
        let id1 = state.generate_unique_window_id();
        let id2 = state.generate_unique_window_id();
        // Assert
        assert_ne!(id1, id2);
    }

    #[test]
    // [CDU-WindowLifecycleEventsV1] Window teardown removes native state so `WindowDestroyed` observers don't see stale HWNDs.
    fn remove_window_data_removes_entry() {
        // Arrange
        let (state, window_id, data) = setup_state();
        {
            let mut guard = state.active_windows().write().unwrap();
            guard.insert(window_id, data);
        }
        // Act
        state.remove_window_data(window_id);
        // Assert
        let guard = state.active_windows().read().unwrap();
        assert!(!guard.contains_key(&window_id));
    }

    #[test]
    // [CDU-Control-TreeViewV1][CDU-TreeView-PopulationV1][CDU-TreeView-ItemStateV1][CDU-TreeView-ItemSelectionV1][CDU-Tech-ThreadSafetyV1]
    // Exercising the tree view state accessor proves that item mappings survive concurrent command mutations.
    fn with_treeview_state_mut_preserves_state_on_success() {
        // Arrange
        let (state, window_id, mut data) = setup_state();
        data.register_control_hwnd(
            ControlId::new(1),
            HWND(std::ptr::dangling_mut::<std::ffi::c_void>()),
        );
        data.init_treeview_state();
        {
            let mut guard = state.active_windows().write().unwrap();
            guard.insert(window_id, data);
        }
        // Act
        let result =
            state.with_treeview_state_mut(window_id, ControlId::new(1), |_hwnd, tv_state| {
                tv_state
                    .item_id_to_htreeitem
                    .insert(TreeItemId(7), HTREEITEM(7));
                Ok(())
            });
        // Assert
        assert!(result.is_ok());
        let guard = state.active_windows().read().unwrap();
        let window = guard.get(&window_id).unwrap();
        let tv_state = window.get_treeview_state().expect("treeview state");
        assert!(tv_state.item_id_to_htreeitem.contains_key(&TreeItemId(7)));
    }

    #[test]
    // [CDU-IdempotentCommandsV1][CDU-Tech-ErrorHandlingV1] TreeView commands fail gracefully without corrupting state when handlers return errors.
    fn with_treeview_state_mut_preserves_state_on_error() {
        // Arrange
        let (state, window_id, mut data) = setup_state();
        data.register_control_hwnd(
            ControlId::new(1),
            HWND(std::ptr::dangling_mut::<std::ffi::c_void>()),
        );
        data.init_treeview_state();
        {
            let mut guard = state.active_windows().write().unwrap();
            guard.insert(window_id, data);
        }
        // Act
        let result =
            state.with_treeview_state_mut(window_id, ControlId::new(1), |_hwnd, tv_state| {
                tv_state
                    .item_id_to_htreeitem
                    .insert(TreeItemId(9), HTREEITEM(9));
                Err(PlatformError::OperationFailed("fail".into()))
            });
        // Assert
        assert!(result.is_err());
        let guard = state.active_windows().read().unwrap();
        let window = guard.get(&window_id).unwrap();
        let tv_state = window.get_treeview_state().expect("treeview state");
        assert!(tv_state.item_id_to_htreeitem.contains_key(&TreeItemId(9)));
    }

    #[test]
    // [CDU-AppQuitV1] As long as windows remain, the quit path defers posting WM_QUIT.
    fn should_quit_on_last_window_close_false_when_windows_exist() {
        // Arrange
        let (state, window_id, data) = setup_state();
        {
            let mut guard = state.active_windows().write().unwrap();
            guard.insert(window_id, data);
        }
        // Act
        let result = state.should_quit_on_last_window_close();
        // Assert
        assert!(!result);
    }

    #[test]
    // [CDU-AppQuitV1] When the final window disappears, the quit logic triggers immediately.
    fn should_quit_on_last_window_close_true_when_no_windows() {
        // Arrange
        let (state, _, _) = setup_state();
        // Act
        let result = state.should_quit_on_last_window_close();
        // Assert
        assert!(result);
    }
}
