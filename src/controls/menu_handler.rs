/*
 * Encapsulates Win32-specific menu creation and command routing.
 * This handler isolates menu logic so that other platform components
 * remain decoupled from raw Win32 menu APIs.
 *
 * The concrete implementations will be added as menu functionality
 * is migrated from `command_executor` and `window_common` in later steps.
 */

use crate::{
    app::Win32ApiInternalState,
    error::{PlatformError, Result as PlatformResult},
    types::{AppEvent, MenuItemConfig, WindowId},
    window_common::NativeWindowData,
};

use std::sync::Arc;

use windows::{
    Win32::{
        Foundation::{GetLastError, HWND},
        UI::WindowsAndMessaging::{
            AppendMenuW, CreateMenu, CreatePopupMenu, DestroyMenu, HMENU, MF_POPUP, MF_STRING,
            SetMenu,
        },
    },
    core::HSTRING,
};
/*
 * Handles the `CreateMainMenu` command by constructing the native menu
 * structure for the given window.
 *
 * The menu items are registered with `NativeWindowData` so that
 * subsequent `WM_COMMAND` messages can be translated into
 * `MenuAction` values.
 */
pub(crate) fn handle_create_main_menu_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    menu_items: Vec<MenuItemConfig>,
) -> PlatformResult<()> {
    log::debug!("MenuHandler: creating main menu for WinID {window_id:?}");

    let h_main_menu = unsafe { CreateMenu()? };

    let hwnd_owner = internal_state.with_window_data_write(window_id, |window_data| {
        let hwnd = window_data.get_hwnd();
        if hwnd.is_invalid() {
            log::warn!(
                "MenuHandler: HWND not yet valid for WindowId {window_id:?} during menu creation."
            );
            return Err(PlatformError::InvalidHandle(format!(
                "HWND not yet valid for WindowId {window_id:?} during menu creation"
            )));
        }

        for item_config in &menu_items {
            unsafe { add_menu_item_recursive_impl(h_main_menu, item_config, window_data)? };
        }
        Ok(hwnd)
    })?;

    if unsafe { SetMenu(hwnd_owner, Some(h_main_menu)) }.is_err() {
        let last_error = unsafe { GetLastError() };
        unsafe {
            DestroyMenu(h_main_menu).unwrap_or_default();
        }
        log::error!(
            "MenuHandler: SetMenu failed for main menu on WindowId {window_id:?}: {last_error:?}"
        );
        return Err(PlatformError::OperationFailed(format!(
            "SetMenu failed for main menu on WindowId {window_id:?}: {last_error:?}"
        )));
    }

    log::debug!("MenuHandler: main menu created and set for WindowId {window_id:?}");
    Ok(())
}

/*
 * Internal helper for recursively adding menu items to a parent menu.
 *
 * Each command item registers its action in the window's menu map,
 * while popups are constructed recursively.
 */
pub(crate) unsafe fn add_menu_item_recursive_impl(
    parent_menu_handle: HMENU,
    item_config: &MenuItemConfig,
    window_data: &mut NativeWindowData,
) -> PlatformResult<()> {
    if item_config.children.is_empty() {
        if let Some(action) = item_config.action {
            let generated_id = window_data.register_menu_action(action);
            unsafe {
                AppendMenuW(
                    parent_menu_handle,
                    MF_STRING,
                    generated_id as usize,
                    &HSTRING::from(item_config.text.as_str()),
                )?
            };
        } else {
            log::warn!(
                "MenuHandler: menu item '{}' has no children and no action.",
                item_config.text
            );
        }
    } else {
        let h_submenu = unsafe { CreatePopupMenu()? };
        for child_config in &item_config.children {
            unsafe { add_menu_item_recursive_impl(h_submenu, child_config, window_data)? };
        }
        unsafe {
            AppendMenuW(
                parent_menu_handle,
                MF_POPUP,
                h_submenu.0 as usize,
                &HSTRING::from(item_config.text.as_str()),
            )?
        };
    }
    Ok(())
}

/*
 * Extracted handling of menu-triggered `WM_COMMAND` messages.
 *
 * The command ID is looked up in the window's menu map and, if found,
 * converted into an `AppEvent::MenuActionClicked`.
 */
pub(crate) fn handle_wm_command_for_menu(
    window_id: WindowId,
    command_id: i32,
    _hwnd_menu_owner: HWND,
    internal_state: &Arc<Win32ApiInternalState>,
) -> Option<AppEvent> {
    let menu_action_result =
        internal_state.with_window_data_read(window_id, |wd| Ok(wd.get_menu_action(command_id)));

    match menu_action_result {
        Ok(Some(action)) => {
            log::debug!("Menu action {action:?} (ID {command_id}) for WinID {window_id:?}.");
            Some(AppEvent::MenuActionClicked { action })
        }
        Ok(None) => {
            log::warn!(
                "WM_COMMAND (Menu/Accel) for unknown ID {command_id} in WinID {window_id:?}."
            );
            None
        }
        Err(e) => {
            log::error!(
                "Failed to access window data for WM_COMMAND (Menu/Accel) in WinID {window_id:?}: {e:?}"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        WindowId,
        app::Win32ApiInternalState,
        types::{MenuAction, MenuItemConfig},
        window_common::NativeWindowData,
    };
    use std::sync::Arc;
    use windows::Win32::UI::WindowsAndMessaging::{CreateMenu, DestroyMenu};

    // Arrange common environment for menu tests
    fn setup_test_env() -> (Arc<Win32ApiInternalState>, WindowId, NativeWindowData) {
        let internal_state = Win32ApiInternalState::new("TestMenu".to_string()).unwrap();
        let window_id = internal_state.generate_unique_window_id();
        let native_window_data = NativeWindowData::new(window_id);
        (internal_state, window_id, native_window_data)
    }

    #[test]
    fn test_add_menu_item_recursive_impl_builds_map_and_ids() {
        // Arrange
        let (_state, _window_id, mut native_data) = setup_test_env();
        let menu_items = vec![
            MenuItemConfig {
                action: Some(MenuAction::LoadProfile),
                text: "Load".into(),
                children: vec![],
            },
            MenuItemConfig {
                action: None,
                text: "File".into(),
                children: vec![MenuItemConfig {
                    action: Some(MenuAction::SaveProfileAs),
                    text: "Save As".into(),
                    children: vec![],
                }],
            },
            MenuItemConfig {
                action: Some(MenuAction::RefreshFileList),
                text: "Refresh".into(),
                children: vec![],
            },
        ];

        // Act
        unsafe {
            let h_main_menu = CreateMenu().expect("create menu");
            for cfg in &menu_items {
                add_menu_item_recursive_impl(h_main_menu, cfg, &mut native_data).unwrap();
            }
            DestroyMenu(h_main_menu).expect("destroy menu");
        }

        // Assert
        assert_eq!(native_data.menu_action_count(), 3);
        assert_eq!(native_data.get_next_menu_item_id_counter(), 30003);
        let actions: Vec<MenuAction> = native_data.iter_menu_actions().map(|(_, a)| *a).collect();
        assert!(actions.contains(&MenuAction::LoadProfile));
        assert!(actions.contains(&MenuAction::SaveProfileAs));
        assert!(actions.contains(&MenuAction::RefreshFileList));
    }
}
