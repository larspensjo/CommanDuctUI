/*
 * This module provides platform-specific (Win32) implementations for TreeView control
 * operations. It handles the creation, population, manipulation, custom drawing,
 * and event handling for native TreeView items based on platform-agnostic commands
 * and descriptors. It also defines the internal state (`TreeViewInternalState`)
 * required to manage a TreeView control.
 *
 * It centralizes all TreeView-related Win32 API interactions, making other parts
 * of the platform layer (like command_executor and window_common) less coupled
 * to specific TreeView details.
 */
use crate::app::Win32ApiInternalState;
use crate::controls::styling_handler;
use crate::error::{PlatformError, Result as PlatformResult};
use crate::styling::StyleId;
use crate::types::{AppEvent, CheckState, ControlId, TreeItemDescriptor, TreeItemId, WindowId};
use crate::window_common::ControlKind;

use windows::{
    Win32::{
        Foundation::{GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{HFONT, HGDIOBJ, InvalidateRect, ScreenToClient, SelectObject},
        UI::Controls::{
            CDDS_ITEMPOSTPAINT, CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDRF_DODEFAULT, CDRF_NEWFONT,
            CDRF_NOTIFYITEMDRAW, CDRF_NOTIFYPOSTPAINT, HTREEITEM, NMHDR, NMTVCUSTOMDRAW,
            TVGN_CARET, TVHITTESTINFO, TVHT_ONITEMLABEL, TVHT_ONITEMSTATEICON, TVI_LAST,
            TVIF_CHILDREN, TVIF_PARAM, TVIF_STATE, TVIF_TEXT, TVINSERTSTRUCTW, TVINSERTSTRUCTW_0,
            TVIS_STATEIMAGEMASK, TVITEMEXW, TVITEMEXW_CHILDREN, TVM_DELETEITEM, TVM_GETITEMRECT,
            TVM_GETITEMW, TVM_HITTEST, TVM_INSERTITEMW, TVM_SELECTITEM, TVM_SETITEMW,
            TVS_CHECKBOXES, TVS_HASBUTTONS, TVS_HASLINES, TVS_LINESATROOT, TVS_SHOWSELALWAYS,
            WC_TREEVIEWW,
        },
        UI::WindowsAndMessaging::*,
    },
    core::PWSTR,
};

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;

/*
 * --- DEACTIVATED ---
 * The values below supported the manual blue-dot drawing for "New" items. They
 * are preserved for future experimentation but are no longer part of the active
 * rendering path now that font styling drives the indicator.
 *
 * const CIRCLE_DIAMETER: i32 = 6;
 * const CIRCLE_COLOR_BLUE: windows::Win32::Foundation::COLORREF =
 *     windows::Win32::Foundation::COLORREF(0x00FF0000); // BGR format for Blue
 */

/*
 * Holds internal state specific to a TreeView control instance.
 * This includes mappings between application-defined `TreeItemId`s and native
 * `HTREEITEM` handles, which are essential for translating commands and events.
 */
#[derive(Debug)]
pub(crate) struct TreeViewInternalState {
    pub(crate) item_id_to_htreeitem: HashMap<TreeItemId, HTREEITEM>,
    pub(crate) htreeitem_to_item_id: HashMap<isize, TreeItemId>,
    pub(crate) style_overrides: HashMap<TreeItemId, StyleId>,
}

impl TreeViewInternalState {
    pub(crate) fn new() -> Self {
        Self {
            item_id_to_htreeitem: HashMap::new(),
            htreeitem_to_item_id: HashMap::new(),
            style_overrides: HashMap::new(),
        }
    }

    fn clear_items_impl(&mut self, hwnd_treeview: HWND) {
        // [CDU-TreeView-PopulationV1] Clearing all nodes guarantees PopulateTreeView commands rebuild the hierarchy without leftovers.
        if hwnd_treeview.is_invalid() {
            log::error!("TreeViewInternalState::clear_items_impl called with invalid HWND");
            return;
        }
        unsafe {
            SendMessageW(
                hwnd_treeview,
                TVM_DELETEITEM,
                Some(WPARAM(0)),
                Some(LPARAM(HTREEITEM(0).0)), // Passing TVI_ROOT (0) or NULL deletes all items
            );
        }
        self.item_id_to_htreeitem.clear();
        self.htreeitem_to_item_id.clear();
        self.style_overrides.clear();
        log::debug!("TreeViewInternalState::clear_items_impl completed for HWND {hwnd_treeview:?}");
    }

    fn add_item_recursive_impl(
        &mut self,
        hwnd_treeview: HWND,
        h_parent_native: HTREEITEM,
        item_desc: &TreeItemDescriptor,
    ) -> PlatformResult<()> {
        if hwnd_treeview.is_invalid() {
            log::error!("TreeViewInternalState::add_item_recursive_impl called with invalid HWND");
            return Err(PlatformError::InvalidHandle(
                "Invalid TreeView HWND".to_string(),
            ));
        }

        let mut text_buffer: Vec<u16> = item_desc.text.encode_utf16().collect();
        text_buffer.push(0); // Null terminator

        // Determine the state image index for checkbox (1-based: 1 for unchecked, 2 for checked)
        let image_index = match item_desc.state {
            CheckState::Checked => 2,
            CheckState::Unchecked => 1,
        };

        let tv_item = TVITEMEXW {
            mask: TVIF_TEXT | TVIF_PARAM | TVIF_CHILDREN,
            hItem: HTREEITEM::default(), // Will be filled by the system if successful
            pszText: PWSTR(text_buffer.as_mut_ptr()),
            cchTextMax: text_buffer.len() as i32,
            lParam: LPARAM(item_desc.id.0 as isize), // Store app-specific TreeItemId
            cChildren: TVITEMEXW_CHILDREN(if item_desc.is_folder { 1 } else { 0 }), // Hint if it has children
            ..Default::default()
        };

        let tv_insert_struct = TVINSERTSTRUCTW {
            hParent: h_parent_native,
            hInsertAfter: TVI_LAST, // Insert at the end of the parent's children
            Anonymous: TVINSERTSTRUCTW_0 { itemex: tv_item },
        };

        let h_current_item_native = HTREEITEM(
            unsafe {
                SendMessageW(
                    hwnd_treeview,
                    TVM_INSERTITEMW,
                    Some(WPARAM(0)),
                    Some(LPARAM(&tv_insert_struct as *const _ as isize)),
                )
            }
            .0,
        );

        if h_current_item_native.0 == 0 {
            // TVM_INSERTITEMW returns NULL on failure
            return Err(PlatformError::ControlCreationFailed(format!(
                "Failed to insert TreeView item '{}': {:?}",
                item_desc.text,
                unsafe { GetLastError() }
            )));
        }

        self.item_id_to_htreeitem
            .insert(item_desc.id, h_current_item_native);
        self.htreeitem_to_item_id
            .insert(h_current_item_native.0, item_desc.id);
        if let Some(style_id) = item_desc.style_override {
            self.style_overrides.insert(item_desc.id, style_id);
        }

        // Explicitly set the state after insertion. This ensures the built-in
        // state image list for checkboxes is attached before we request a
        // particular check state.
        // [CDU-TreeView-ItemStateV1] Programmatic checkbox state is applied immediately so UI and model stay in sync.
        let mut tv_item_update = TVITEMEXW {
            mask: TVIF_STATE,
            hItem: h_current_item_native,
            state: (image_index as u32) << 12,
            stateMask: TVIS_STATEIMAGEMASK.0,
            ..Default::default()
        };

        unsafe {
            SendMessageW(
                hwnd_treeview,
                TVM_SETITEMW,
                Some(WPARAM(0)),
                Some(LPARAM(&mut tv_item_update as *mut _ as isize)),
            );
        }

        // Recursively add children if this item is a folder and has children
        if item_desc.is_folder && !item_desc.children.is_empty() {
            for child_desc in &item_desc.children {
                self.add_item_recursive_impl(hwnd_treeview, h_current_item_native, child_desc)?;
            }
        }
        Ok(())
    }

    fn style_override_for(&self, item_id: &TreeItemId) -> Option<StyleId> {
        self.style_overrides.get(item_id).copied()
    }
}

/*
 * Handles the creation of a native TreeView control.
 * This function uses a read-create-write pattern to minimize lock contention.
 * It first checks for conflicts using a read lock, then creates the native
 * control, and finally uses a write lock to register the new control and its state.
 * [CDU-Control-TreeViewV1] TreeView creation ties the logical `ControlId` to the native HWND and seeds the checkbox/selection tracking maps.
 */
pub(crate) fn handle_create_treeview_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
) -> PlatformResult<()> {
    log::debug!(
        "TreeViewHandler: handle_create_treeview_command for WinID {window_id:?}, ParentID {:?}, ControlID {}",
        parent_control_id.as_ref().map(|id| id.raw()),
        control_id.raw()
    );

    // Phase 1: Read-only pre-checks.
    let hwnd_parent_for_creation =
        internal_state.with_window_data_read(window_id, |window_data| {
            if window_data.has_control(control_id) || window_data.has_treeview_state() {
                return Err(PlatformError::ControlCreationFailed(format!(
                    "TreeView with ID {} or existing treeview state already present for window {window_id:?}",
                    control_id.raw()
                )));
            }

            let hwnd = match parent_control_id {
                Some(id) => window_data.get_control_hwnd(id).ok_or_else(|| {
                    PlatformError::InvalidHandle(format!(
                        "Parent control with ID {} not found for CreateTreeView in WinID {window_id:?}",
                        id.raw()
                    ))
                })?,
                None => window_data.get_hwnd(),
            };

            if hwnd.is_invalid() {
                return Err(PlatformError::InvalidHandle(format!(
                    "Parent HWND for CreateTreeView is invalid (WinID: {window_id:?})"
                )));
            }
            Ok(hwnd)
        })?;

    // Phase 2: Create the window without holding a lock.
    let h_instance_for_creation = internal_state.h_instance();
    let tvs_style = WINDOW_STYLE(
        TVS_HASLINES | TVS_LINESATROOT | TVS_HASBUTTONS | TVS_SHOWSELALWAYS | TVS_CHECKBOXES,
    );
    let combined_style = WS_CHILD | WS_VISIBLE | WS_BORDER | tvs_style;
    let hwnd_tv = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WC_TREEVIEWW, // Standard class name for TreeView
            None,         // No window text/title for a control
            combined_style,
            0,
            0,
            10,
            10, // Dummy position/size, layout rules will adjust
            Some(hwnd_parent_for_creation),
            Some(HMENU(control_id.raw() as *mut _)), // Use logical ID for HMENU
            Some(h_instance_for_creation),
            None, // No extra creation parameters
        )?
    };

    // Phase 3: Acquire write lock to update NativeWindowData.
    internal_state.with_window_data_write(window_id, |window_data| {
        // Re-check for race conditions.
        if window_data.has_control(control_id) || window_data.has_treeview_state() {
            log::warn!(
                "TreeViewHandler: TreeView (ID {}) created concurrently. Destroying new one.",
                control_id.raw()
            );
            unsafe { DestroyWindow(hwnd_tv).ok() };
            return Err(PlatformError::ControlCreationFailed(format!(
                "TreeView with ID {} was concurrently created for window {window_id:?}",
                control_id.raw()
            )));
        }

        window_data.register_control_hwnd(control_id, hwnd_tv);
        window_data.register_control_kind(control_id, ControlKind::TreeView);
        window_data.init_treeview_state();
        log::debug!(
            "TreeViewHandler: Created TreeView (ID {}) for window {window_id:?} with HWND {hwnd_tv:?}",
            control_id.raw()
        );
        Ok(())
    })
}

/*
 * Populates a TreeView control with a given set of item descriptors.
 * This function clears any existing items in the TreeView and then recursively
 * adds the new items. It uses the specialized `with_treeview_state_mut` helper
 * to ensure the main window map is not locked during the potentially lengthy
 * population process.
 */
pub(crate) fn populate_treeview(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    items: Vec<TreeItemDescriptor>,
) -> PlatformResult<()> {
    log::debug!(
        "TreeViewHandler: populate_treeview called for WinID {window_id:?}, ControlID {}",
        control_id.raw()
    );

    internal_state.with_treeview_state_mut(window_id, control_id, |hwnd_treeview, tv_state| {
        log::debug!(
            "TreeViewHandler: Populating TreeView (HWND {hwnd_treeview:?}). Clearing existing items."
        );
        tv_state.clear_items_impl(hwnd_treeview);

        for item_desc in items {
            tv_state.add_item_recursive_impl(hwnd_treeview, HTREEITEM(0), &item_desc)?;
        }

        log::debug!(
            "TreeViewHandler: Finished populating TreeView (HWND {hwnd_treeview:?})."
        );
        Ok(())
    })
}

/*
 * Updates the visual state (specifically the checkbox) of a single TreeView item.
 * It maps the application-defined `TreeItemId` to its native `HTREEITEM` and sends
 * a `TVM_SETITEMW` message to change its state image index.
 */
pub(crate) fn update_treeview_item_visual_state(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    item_id: TreeItemId,
    new_check_state: CheckState,
) -> PlatformResult<()> {
    log::debug!(
        "TreeViewHandler: update_treeview_item_visual_state for WinID {window_id:?}, ControlID {}, ItemID {item_id:?}",
        control_id.raw()
    );

    // Get all necessary handles and data within a single read lock.
    let (hwnd_treeview, h_item_native) =
        internal_state.with_window_data_read(window_id, |window_data| {
            let hwnd = window_data.get_control_hwnd(control_id).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "TreeView HWND not found for ControlID {}",
                    control_id.raw()
                ))
            })?;

            let tv_state = window_data.get_treeview_state().ok_or_else(|| {
                PlatformError::OperationFailed(format!(
                    "No TreeView state exists in window {window_id:?}"
                ))
            })?;

            let h_item = tv_state
                .item_id_to_htreeitem
                .get(&item_id)
                .copied()
                .ok_or_else(|| {
                    PlatformError::InvalidHandle(format!("TreeItemId {item_id:?} not found"))
                })?;

            Ok((hwnd, h_item))
        })?;

    if hwnd_treeview.is_invalid() {
        return Err(PlatformError::InvalidHandle("Invalid TreeView HWND".into()));
    }

    let image_index = match new_check_state {
        CheckState::Checked => 2,   // Index for checked state image
        CheckState::Unchecked => 1, // Index for unchecked state image
    };

    let mut tv_item_update = TVITEMEXW {
        mask: TVIF_STATE,
        hItem: h_item_native,
        state: (image_index as u32) << 12, // State image index is bits 12-15 of state
        stateMask: TVIS_STATEIMAGEMASK.0,
        ..Default::default()
    };

    let send_result = unsafe {
        SendMessageW(
            hwnd_treeview,
            TVM_SETITEMW,
            Some(WPARAM(0)),
            Some(LPARAM(&mut tv_item_update as *mut _ as isize)),
        )
    };

    if send_result.0 == 0 {
        let last_error = unsafe { GetLastError() };
        return Err(PlatformError::OperationFailed(format!(
            "TVM_SETITEMW failed for item {item_id:?}: {last_error:?}"
        )));
    }
    Ok(())
}

/*
 * Updates the rendered text for a single TreeView item. This reuses the stored
 * `HTREEITEM` mapping to send a `TVM_SETITEMW` call with a new UTF-16 buffer,
 * allowing the application logic to append or remove the indicator glyph without
 * rebuilding the entire control.
 */
pub(crate) fn update_treeview_item_text(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    item_id: TreeItemId,
    text: String,
) -> PlatformResult<()> {
    log::debug!(
        "TreeViewHandler: update_treeview_item_text for WinID {window_id:?}, ControlID {}, ItemID {item_id:?}",
        control_id.raw()
    );

    let (hwnd_treeview, h_item_native) =
        internal_state.with_window_data_read(window_id, |window_data| {
            let hwnd = window_data.get_control_hwnd(control_id).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "TreeView HWND not found for ControlID {}",
                    control_id.raw()
                ))
            })?;

            let tv_state = window_data.get_treeview_state().ok_or_else(|| {
                PlatformError::OperationFailed(format!(
                    "No TreeView state exists in window {window_id:?}"
                ))
            })?;

            let h_item = tv_state
                .item_id_to_htreeitem
                .get(&item_id)
                .copied()
                .ok_or_else(|| {
                    PlatformError::InvalidHandle(format!("TreeItemId {item_id:?} not found"))
                })?;

            Ok((hwnd, h_item))
        })?;

    if hwnd_treeview.is_invalid() {
        return Err(PlatformError::InvalidHandle("Invalid TreeView HWND".into()));
    }

    let mut text_buffer: Vec<u16> = text.encode_utf16().collect();
    text_buffer.push(0);

    let mut tv_item_update = TVITEMEXW {
        mask: TVIF_TEXT,
        hItem: h_item_native,
        pszText: PWSTR(text_buffer.as_mut_ptr()),
        cchTextMax: text_buffer.len() as i32,
        ..Default::default()
    };

    let send_result = unsafe {
        SendMessageW(
            hwnd_treeview,
            TVM_SETITEMW,
            Some(WPARAM(0)),
            Some(LPARAM(&mut tv_item_update as *mut _ as isize)),
        )
    };

    if send_result.0 == 0 {
        let last_error = unsafe { GetLastError() };
        return Err(PlatformError::OperationFailed(format!(
            "TVM_SETITEMW (text update) failed for item {item_id:?}: {last_error:?}"
        )));
    }
    Ok(())
}

/*
 * Handles the TVN_ITEMCHANGEDW notification for a TreeView.
 * This notification is sent for various item state changes, but this handler
 * currently only logs the event.
 */
pub(crate) fn handle_treeview_itemchanged_notification(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    _lparam: LPARAM,
    control_id_from_notify: ControlId,
) -> Option<AppEvent> {
    log::trace!(
        "TreeViewHandler: TVN_ITEMCHANGEDW received for WinID {window_id:?}, ControlID {}",
        control_id_from_notify.raw()
    );
    // Check if TreeView state exists for this window.
    if let Ok(false) =
        internal_state.with_window_data_read(window_id, |wd| Ok(wd.has_treeview_state()))
    {
        log::warn!("Received TVN_ITEMCHANGEDW for a window without treeview state.");
        return None;
    }

    // `lparam` could be used here to get more details if needed.
    // let nmtv = unsafe { &*(lparam.0 as *const NMTREEVIEWW) };
    None // No AppEvent generated from this notification directly for now
}

/*
 * Executes the `RedrawTreeItem` command by invalidating the rectangle of a specific item.
 * This function retrieves the native `HTREEITEM` for the given `TreeItemId` and
 * uses `TVM_GETITEMRECT` to find its bounding box, then forces a repaint.
 */
pub(crate) fn handle_redraw_tree_item_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    item_id: TreeItemId,
) -> PlatformResult<()> {
    log::debug!(
        "TreeViewHandler: handle_redraw_tree_item_command for WinID {window_id:?}, ItemID {item_id:?}"
    );

    let (hwnd_treeview, htreeitem) =
        internal_state.with_window_data_read(window_id, |window_data| {
            let hwnd = window_data.get_control_hwnd(control_id).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "TreeView control (ID {}) not found",
                    control_id.raw()
                ))
            })?;

            let tv_state = window_data.get_treeview_state().ok_or_else(|| {
                PlatformError::OperationFailed(format!(
                    "TreeView state not found for WinID {window_id:?}"
                ))
            })?;

            let h_item = tv_state
                .item_id_to_htreeitem
                .get(&item_id)
                .copied()
                .ok_or_else(|| {
                    PlatformError::InvalidHandle(format!(
                        "HTREEITEM not found for ItemID {item_id:?}"
                    ))
                })?;
            Ok((hwnd, h_item))
        })?;

    if hwnd_treeview.is_invalid() {
        return Err(PlatformError::InvalidHandle("Invalid TreeView HWND".into()));
    }

    let mut item_rect = RECT::default();
    unsafe {
        *((&mut item_rect as *mut RECT) as *mut HTREEITEM) = htreeitem;
    }

    let get_rect_success = unsafe {
        SendMessageW(
            hwnd_treeview,
            TVM_GETITEMRECT,
            Some(WPARAM(0)), // FALSE for whole item
            Some(LPARAM(&mut item_rect as *mut _ as isize)),
        )
    };

    if get_rect_success.0 != 0 {
        unsafe {
            _ = InvalidateRect(Some(hwnd_treeview), Some(&item_rect), true);
        }
    } else {
        log::warn!(
            "TVM_GETITEMRECT failed for item ID {:?}, invalidating whole control. Error: {:?}",
            item_id,
            unsafe { GetLastError() }
        );
        unsafe {
            _ = InvalidateRect(Some(hwnd_treeview), None, true);
        }
    }
    Ok(())
}

fn get_treeview_hwnd(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
) -> PlatformResult<HWND> {
    internal_state.with_window_data_read(window_id, |window_data| {
        let hwnd = window_data.get_control_hwnd(control_id).ok_or_else(|| {
            PlatformError::InvalidHandle(format!(
                "Control ID {} not found in WinID {window_id:?}",
                control_id.raw()
            ))
        })?;

        if hwnd.is_invalid() {
            return Err(PlatformError::InvalidHandle(format!(
                "HWND for control ID {} is invalid",
                control_id.raw()
            )));
        }
        Ok(hwnd)
    })
}

pub(crate) fn expand_visible_tree_items(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
) -> PlatformResult<()> {
    log::debug!(
        "TreeViewHandler: expand_visible_tree_items for WinID {window_id:?}, ControlID {}",
        control_id.raw()
    );
    let hwnd_treeview = get_treeview_hwnd(internal_state, window_id, control_id)?;

    use windows::Win32::UI::Controls::{
        TVE_EXPAND, TVGN_FIRSTVISIBLE, TVGN_NEXTVISIBLE, TVM_EXPAND, TVM_GETNEXTITEM,
    };

    unsafe {
        let mut item = SendMessageW(
            hwnd_treeview,
            TVM_GETNEXTITEM,
            Some(WPARAM(TVGN_FIRSTVISIBLE as usize)),
            Some(LPARAM(0)),
        );

        while item.0 != 0 {
            _ = SendMessageW(
                hwnd_treeview,
                TVM_EXPAND,
                Some(WPARAM(TVE_EXPAND.0 as usize)),
                Some(LPARAM(item.0)),
            );

            item = SendMessageW(
                hwnd_treeview,
                TVM_GETNEXTITEM,
                Some(WPARAM(TVGN_NEXTVISIBLE as usize)),
                Some(LPARAM(item.0)),
            );
        }
    }

    Ok(())
}

pub(crate) fn expand_all_tree_items(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
) -> PlatformResult<()> {
    log::debug!(
        "TreeViewHandler: expand_all_tree_items for WinID {window_id:?}, ControlID {}",
        control_id.raw()
    );
    let hwnd_treeview = get_treeview_hwnd(internal_state, window_id, control_id)?;

    use windows::Win32::UI::Controls::{
        TVE_EXPAND, TVGN_CHILD, TVGN_NEXT, TVGN_ROOT, TVM_EXPAND, TVM_GETNEXTITEM,
    };

    unsafe fn recurse(hwnd: HWND, item: HTREEITEM) {
        if item.0 == 0 {
            return;
        }
        unsafe {
            let _ = SendMessageW(
                hwnd,
                TVM_EXPAND,
                Some(WPARAM(TVE_EXPAND.0 as usize)),
                Some(LPARAM(item.0)),
            );
        }
        let mut child = unsafe {
            SendMessageW(
                hwnd,
                TVM_GETNEXTITEM,
                Some(WPARAM(TVGN_CHILD as usize)),
                Some(LPARAM(item.0)),
            )
        };
        unsafe {
            while child.0 != 0 {
                recurse(hwnd, HTREEITEM(child.0));
                child = SendMessageW(
                    hwnd,
                    TVM_GETNEXTITEM,
                    Some(WPARAM(TVGN_NEXT as usize)),
                    Some(LPARAM(child.0)),
                );
            }
        }
    }

    unsafe {
        let mut root = SendMessageW(
            hwnd_treeview,
            TVM_GETNEXTITEM,
            Some(WPARAM(TVGN_ROOT as usize)),
            Some(LPARAM(0)),
        );
        while root.0 != 0 {
            recurse(hwnd_treeview, HTREEITEM(root.0));
            root = SendMessageW(
                hwnd_treeview,
                TVM_GETNEXTITEM,
                Some(WPARAM(TVGN_NEXT as usize)),
                Some(LPARAM(root.0)),
            );
        }
    }

    Ok(())
}

/*
 * Ensures the operating system highlights the TreeView row for a provided item.
 * This centralizes the conversion from logical `TreeItemId` to native `HTREEITEM`
 * and issues the `TVM_SELECTITEM` message so layout code can simply enqueue a command.
 * [CDU-TreeView-ItemSelectionV1] Selection commands keep the highlighted row aligned with the app-provided `TreeItemId`.
 */
pub(crate) fn set_treeview_selection(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    item_id: TreeItemId,
) -> PlatformResult<()> {
    log::debug!(
        "TreeViewHandler: set_treeview_selection for WinID {window_id:?}, ControlID {}, ItemID {item_id:?}",
        control_id.raw()
    );

    let (hwnd_treeview, h_item_native) =
        internal_state.with_window_data_read(window_id, |window_data| {
            let hwnd = window_data.get_control_hwnd(control_id).ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "TreeView HWND not found for ControlID {}",
                    control_id.raw()
                ))
            })?;

            let tv_state = window_data.get_treeview_state().ok_or_else(|| {
                PlatformError::OperationFailed(format!(
                    "No TreeView state available for WinID {window_id:?}"
                ))
            })?;

            let h_item = tv_state
                .item_id_to_htreeitem
                .get(&item_id)
                .copied()
                .ok_or_else(|| {
                    PlatformError::InvalidHandle(format!(
                        "TreeItemId {item_id:?} not found when selecting"
                    ))
                })?;
            Ok((hwnd, h_item))
        })?;

    if hwnd_treeview.is_invalid() {
        return Err(PlatformError::InvalidHandle(
            "Invalid TreeView HWND while selecting item".into(),
        ));
    }

    let select_result = unsafe {
        SendMessageW(
            hwnd_treeview,
            TVM_SELECTITEM,
            Some(WPARAM(TVGN_CARET as usize)),
            Some(LPARAM(h_item_native.0)),
        )
    };

    if select_result.0 == 0 {
        log::warn!(
            "TreeViewHandler: TVM_SELECTITEM failed for ItemID {item_id:?} on ControlID {}",
            control_id.raw()
        );
        return Err(PlatformError::OperationFailed(
            "Failed to select TreeView item".into(),
        ));
    }

    Ok(())
}

/*
 * Determines whether a given TreeView item should display the "new item" styling.
 * Relies on the application logic's `UiStateProvider` to keep the decision centralized.
 */
fn is_item_new_for_display(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    tree_item_id: TreeItemId,
) -> bool {
    let provider_opt = internal_state
        .ui_state_provider()
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|weak_handler| weak_handler.upgrade());

    if let Some(handler_arc) = provider_opt
        && let Ok(handler_guard) = handler_arc.lock()
    {
        return handler_guard.is_tree_item_new(window_id, tree_item_id);
    }

    false
}

/*
 * Handles the NM_CUSTOMDRAW notification for a TreeView control.
 * Applies a bold/italic font to "New" items via NM_CUSTOMDRAW, replacing the former
 * hand-drawn blue circle indicator. The old drawing logic is preserved in comments
 * for potential future reference.
 * [CDU-Styling-CustomDrawV1] Custom draw hooks allow TreeView rows to render with per-item fonts and colors supplied by the styling system.
 */
pub(crate) fn handle_nm_customdraw(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    lparam_nmcustomdraw: LPARAM, // This is NMTVCUSTOMDRAW*
    control_id_of_treeview: ControlId,
) -> LRESULT {
    let nmtvcd = unsafe { &mut *(lparam_nmcustomdraw.0 as *mut NMTVCUSTOMDRAW) };

    match nmtvcd.nmcd.dwDrawStage {
        CDDS_PREPAINT => {
            log::trace!(
                "TreeViewHandler NM_CUSTOMDRAW (WinID {window_id:?}/CtrlID {}): CDDS_PREPAINT. Requesting CDRF_NOTIFYITEMDRAW.",
                control_id_of_treeview.raw()
            );
            return LRESULT(CDRF_NOTIFYITEMDRAW as isize);
        }
        CDDS_ITEMPREPAINT => {
            let tree_item_id = TreeItemId(nmtvcd.nmcd.lItemlParam.0 as u64);
            let item_is_new = is_item_new_for_display(internal_state, window_id, tree_item_id);

            let style_override = internal_state
                .with_window_data_read(window_id, |window_data| {
                    Ok(window_data
                        .get_treeview_state()
                        .and_then(|state| state.style_override_for(&tree_item_id)))
                })
                .unwrap_or(None);

            let mut selected_font: Option<HFONT> = None;
            if let Some(style_id) = style_override
                && let Some(style) = internal_state.get_parsed_style(style_id)
            {
                if let Some(color) = style.text_color.as_ref() {
                    nmtvcd.clrText = styling_handler::color_to_colorref(color);
                }
                if let Some(bg) = style.background_color.as_ref() {
                    nmtvcd.clrTextBk = styling_handler::color_to_colorref(bg);
                }
                if let Some(font) = style.font_handle {
                    selected_font = Some(font);
                }
            }

            if selected_font.is_none() && item_is_new {
                let mut indicator_font: Option<HFONT> = internal_state
                    .with_window_data_read(window_id, |window_data| {
                        Ok(window_data.get_treeview_new_item_font())
                    })
                    .unwrap_or(None);

                if indicator_font.is_none()
                    && let Ok(font_opt) =
                        internal_state.with_window_data_write(window_id, |window_data| {
                            window_data.ensure_treeview_new_item_font();
                            Ok(window_data.get_treeview_new_item_font())
                        })
                {
                    indicator_font = font_opt;
                }
                selected_font = indicator_font;
            }

            let mut result: isize = CDRF_DODEFAULT as isize;
            if let Some(font_handle) = selected_font {
                unsafe {
                    SelectObject(nmtvcd.nmcd.hdc, HGDIOBJ(font_handle.0));
                }
                result |= CDRF_NEWFONT as isize;
                result |= CDRF_NOTIFYPOSTPAINT as isize;
            }

            return LRESULT(result);
        }
        CDDS_ITEMPOSTPAINT => {
            let hdc = nmtvcd.nmcd.hdc;
            let hwnd_treeview = nmtvcd.nmcd.hdr.hwndFrom;
            let tree_item_id = TreeItemId(nmtvcd.nmcd.lItemlParam.0 as u64);

            let style_override = internal_state
                .with_window_data_read(window_id, |window_data| {
                    Ok(window_data
                        .get_treeview_state()
                        .and_then(|state| state.style_override_for(&tree_item_id)))
                })
                .unwrap_or(None);

            let needs_font_reset = style_override
                .and_then(|style_id| {
                    internal_state
                        .get_parsed_style(style_id)
                        .and_then(|style| style.font_handle)
                })
                .is_some()
                || is_item_new_for_display(internal_state, window_id, tree_item_id);

            if needs_font_reset {
                /*
                 * --- DEACTIVATED ---
                 * Historical manual drawing code retained for reference:
                 *
                 * let h_item_native = HTREEITEM(nmtvcd.nmcd.dwItemSpec as isize);
                 * let mut item_rect_text_part = RECT::default();
                 * unsafe {
                 *     *((&mut item_rect_text_part as *mut RECT) as *mut HTREEITEM) = h_item_native;
                 * }
                 * let get_rect_success = unsafe {
                 *     SendMessageW(
                 *         hwnd_treeview,
                 *         TVM_GETITEMRECT,
                 *         Some(WPARAM(1)),
                 *         Some(LPARAM(&mut item_rect_text_part as *mut _ as isize)),
                 *     )
                 * };
                 * if get_rect_success.0 != 0 {
                 *     // ellipse drawing omitted
                 * }
                 */
                let default_font_lresult = unsafe {
                    SendMessageW(hwnd_treeview, WM_GETFONT, Some(WPARAM(0)), Some(LPARAM(0)))
                };
                let default_font = HFONT(default_font_lresult.0 as usize as *mut c_void);
                if !default_font.0.is_null() {
                    unsafe {
                        SelectObject(hdc, HGDIOBJ(default_font.0));
                    }
                }
            }
            return LRESULT(CDRF_DODEFAULT as isize);
        }
        _ => {}
    }
    LRESULT(CDRF_DODEFAULT as isize)
}

/*
 * Handles the custom WM_APP_TREEVIEW_CHECKBOX_CLICKED message.
 * This message is posted by the NM_CLICK handler when a click occurs on a TreeView
 * item's state icon (checkbox). This function retrieves the item's current checkbox
 * state and the application-specific TreeItemId, then constructs an
 * AppEvent::TreeViewItemToggledByUser to notify the application logic.
 */
pub(crate) fn handle_wm_app_treeview_checkbox_clicked(
    internal_state: &Arc<Win32ApiInternalState>,
    _parent_hwnd: HWND,
    window_id: WindowId,
    wparam_htreeitem: WPARAM,
    lparam_control_id: LPARAM,
) -> Option<AppEvent> {
    let h_item_clicked = HTREEITEM(wparam_htreeitem.0 as isize);
    let control_id_of_treeview = ControlId::new(lparam_control_id.0 as i32);

    if h_item_clicked.0 == 0 || control_id_of_treeview.raw() == 0 {
        return None;
    }

    let result = internal_state.with_window_data_read(window_id, |window_data| {
        let hwnd_treeview = window_data
            .get_control_hwnd(control_id_of_treeview)
            .ok_or_else(|| PlatformError::InvalidHandle("Control not found".into()))?;
        let tv_state = window_data
            .get_treeview_state()
            .ok_or_else(|| PlatformError::OperationFailed("TreeView state not found".into()))?;

        let mut tv_item_get = TVITEMEXW {
            mask: TVIF_STATE | TVIF_PARAM,
            hItem: h_item_clicked,
            stateMask: TVIS_STATEIMAGEMASK.0,
            ..Default::default()
        };

        if unsafe {
            SendMessageW(
                hwnd_treeview,
                TVM_GETITEMW,
                Some(WPARAM(0)),
                Some(LPARAM(&mut tv_item_get as *mut _ as isize)),
            )
        }
        .0 == 0
        {
            return Err(PlatformError::OperationFailed("TVM_GETITEMW failed".into()));
        }

        let state_image_idx = (tv_item_get.state & TVIS_STATEIMAGEMASK.0) >> 12;
        let new_check_state = if state_image_idx == 2 {
            CheckState::Checked
        } else {
            CheckState::Unchecked
        };

        let app_item_id = if tv_item_get.lParam.0 != 0 {
            TreeItemId(tv_item_get.lParam.0 as u64)
        } else {
            // Fallback to map lookup
            tv_state
                .htreeitem_to_item_id
                .get(&(h_item_clicked.0))
                .copied()
                .ok_or_else(|| PlatformError::InvalidHandle("HTREEITEM not found in map".into()))?
        };

        Ok(AppEvent::TreeViewItemToggledByUser {
            window_id,
            item_id: app_item_id,
            new_state: new_check_state,
        })
    });

    match result {
        Ok(event) => Some(event),
        Err(e) => {
            log::error!("Failed to handle checkbox click for HTREEITEM {h_item_clicked:?}: {e:?}");
            None
        }
    }
}

/*
 * Handles general NM_CLICK notifications for a TreeView.
 * This function's primary purpose is to detect clicks on a TreeView item's state
 * icon (checkbox) and post a custom message for deferred processing.
 */
pub(crate) fn handle_nm_click(
    internal_state: &Arc<Win32ApiInternalState>,
    parent_hwnd: HWND,
    window_id: WindowId,
    nmhdr: &NMHDR,
) -> Option<AppEvent> {
    let hwnd_tv_from_notify = nmhdr.hwndFrom;
    if hwnd_tv_from_notify.is_invalid() {
        return None;
    }

    let control_id_from_notify = ControlId::new(nmhdr.idFrom as i32);

    let mut screen_pt_of_click = POINT::default();
    if unsafe { GetCursorPos(&mut screen_pt_of_click) }.is_err() {
        return None;
    }
    let mut client_pt_for_hittest = screen_pt_of_click;
    if unsafe { !ScreenToClient(hwnd_tv_from_notify, &mut client_pt_for_hittest) }.as_bool() {
        return None;
    }

    let mut tvht_info = TVHITTESTINFO {
        pt: client_pt_for_hittest,
        ..Default::default()
    };

    let h_item_hit = HTREEITEM(
        unsafe {
            SendMessageW(
                hwnd_tv_from_notify,
                TVM_HITTEST,
                Some(WPARAM(0)),
                Some(LPARAM(&mut tvht_info as *mut _ as isize)),
            )
        }
        .0,
    );

    if h_item_hit.0 != 0 && (tvht_info.flags.0 & TVHT_ONITEMSTATEICON.0) != 0 {
        log::debug!(
            "NM_CLICK on state icon detected for HTREEITEM {h_item_hit:?}. Posting deferred message.",
        );
        unsafe {
            if PostMessageW(
                Some(parent_hwnd),
                crate::window_common::WM_APP_TREEVIEW_CHECKBOX_CLICKED,
                WPARAM(h_item_hit.0 as usize),
                LPARAM(control_id_from_notify.raw() as isize),
            )
            .is_err()
            {
                log::error!(
                    "Failed to post WM_APP_TREEVIEW_CHECKBOX_CLICKED message: {:?}",
                    GetLastError()
                );
            }
        }
        return None;
    }

    if h_item_hit.0 == 0 {
        return None;
    }

    if (tvht_info.flags.0 & TVHT_ONITEMLABEL.0) == 0 {
        return None;
    }

    let result = internal_state.with_window_data_read(window_id, |window_data| {
        let tv_state = window_data.get_treeview_state().ok_or_else(|| {
            PlatformError::OperationFailed(format!(
                "TreeView state not found while resolving label click in WinID {window_id:?}"
            ))
        })?;

        tv_state
            .htreeitem_to_item_id
            .get(&(h_item_hit.0))
            .copied()
            .ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "HTREEITEM {h_item_hit:?} missing in map during label click"
                ))
            })
    });

    match result {
        Ok(item_id) => {
            log::debug!(
                "TreeView label click resolved to TreeItemId {item_id:?} for WinID {window_id:?}."
            );
            Some(AppEvent::TreeViewItemSelectionChanged { window_id, item_id })
        }
        Err(err) => {
            log::error!(
                "Failed to resolve TreeView label click to item ID in WinID {window_id:?}: {err:?}"
            );
            None
        }
    }
}
