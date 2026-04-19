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
use crate::styling_primitives::Color;
use crate::types::{
    AppEvent, CheckState, ControlId, TreeItemDescriptor, TreeItemId, TreeItemMarkerKind, WindowId,
};
use crate::win32_cast::i32_from_usize_saturating;
use crate::window_common::{ControlKind, try_enable_dark_mode};

use windows::{
    Win32::{
        Foundation::{GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            CreateSolidBrush, DeleteObject, Ellipse, FillRect, HDC, HFONT, HGDIOBJ, InvalidateRect,
            ScreenToClient, SelectObject,
        },
        UI::Controls::{
            CDDS_ITEMPOSTPAINT, CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDIS_FOCUS, CDIS_SELECTED,
            CDRF_DODEFAULT, CDRF_NEWFONT, CDRF_NOTIFYITEMDRAW, CDRF_NOTIFYPOSTPAINT, HTREEITEM,
            NMHDR, NMTREEVIEWW, NMTVCUSTOMDRAW, TVC_BYKEYBOARD, TVC_BYMOUSE, TVGN_CARET,
            TVHITTESTINFO, TVHT_ONITEMSTATEICON, TVI_LAST, TVIF_CHILDREN, TVIF_PARAM, TVIF_STATE,
            TVIF_TEXT, TVINSERTSTRUCTW, TVINSERTSTRUCTW_0, TVIS_STATEIMAGEMASK, TVITEMEXW,
            TVITEMEXW_CHILDREN, TVM_DELETEITEM, TVM_GETITEMRECT, TVM_GETITEMW, TVM_GETNEXTITEM,
            TVM_HITTEST, TVM_INSERTITEMW, TVM_SELECTITEM, TVM_SETITEMW, TVS_CHECKBOXES,
            TVS_HASBUTTONS, TVS_HASLINES, TVS_LINESATROOT, TVS_SHOWSELALWAYS, WC_TREEVIEWW,
        },
        UI::WindowsAndMessaging::*,
    },
    core::PWSTR,
};

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
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

const MARKER_MIN_DIAMETER: i32 = 12;
const MARKER_MAX_DIAMETER: i32 = 14;
const MARKER_LANE_GAP: i32 = 3;
const STATE_ICON_LANE_WIDTH: i32 = 18;
const SELECTION_ACCENT_WIDTH: i32 = 3;
const MARKER_BORDER: i32 = 1;

/*
 * Holds internal state specific to a TreeView control instance.
 * This includes mappings between application-defined `TreeItemId`s and native
 * `HTREEITEM` handles, which are essential for translating commands and events.
 */
#[derive(Debug)]
pub(crate) struct TreeViewInternalState {
    pub(crate) item_id_to_htreeitem: HashMap<TreeItemId, HTREEITEM>,
    pub(crate) htreeitem_to_item_id: HashMap<isize, TreeItemId>,
    pub(crate) check_states: HashMap<TreeItemId, CheckState>,
    pub(crate) style_overrides: HashMap<TreeItemId, StyleId>,
}

impl TreeViewInternalState {
    pub(crate) fn new() -> Self {
        Self {
            item_id_to_htreeitem: HashMap::new(),
            htreeitem_to_item_id: HashMap::new(),
            check_states: HashMap::new(),
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
        self.check_states.clear();
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

        let tv_item = TVITEMEXW {
            mask: TVIF_TEXT | TVIF_PARAM | TVIF_CHILDREN,
            hItem: HTREEITEM::default(), // Will be filled by the system if successful
            pszText: PWSTR(text_buffer.as_mut_ptr()),
            cchTextMax: i32_from_usize_saturating(text_buffer.len()),
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
        self.check_states.insert(item_desc.id, item_desc.state);
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
            state: treeview_state_image_mask(item_desc.state),
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

    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) || window_data.has_treeview_state() {
            log::warn!(
                "TreeViewHandler: TreeView (ID {}) created concurrently. Destroying new one.",
                control_id.raw()
            );
            return Err(PlatformError::ControlCreationFailed(format!(
                "TreeView with ID {} or existing treeview state already present for window {window_id:?}",
                control_id.raw()
            )));
        }
        window_data.register_control_kind(control_id, ControlKind::TreeView);
        Ok(())
    })?;

    // Phase 2: Create the window without holding a lock.
    let h_instance_for_creation = internal_state.h_instance();
    let tvs_style = WINDOW_STYLE(
        TVS_HASLINES | TVS_LINESATROOT | TVS_HASBUTTONS | TVS_SHOWSELALWAYS | TVS_CHECKBOXES,
    );
    let combined_style = WS_CHILD | WS_VISIBLE | WS_BORDER | tvs_style;
    let hwnd_tv = unsafe {
        match CreateWindowExW(
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
        .get_parsed_style(StyleId::MainWindowBackground)
        .is_some()
    {
        try_enable_dark_mode(hwnd_tv);
    }

    // Phase 3: Acquire write lock to update NativeWindowData.
    internal_state.with_window_data_write(window_id, |window_data| {
        // Re-check for race conditions.
        if window_data.has_control(control_id) || window_data.has_treeview_state() {
            log::warn!(
                "TreeViewHandler: TreeView (ID {}) created concurrently. Destroying new one.",
                control_id.raw()
            );
            unsafe { DestroyWindow(hwnd_tv).ok() };
            window_data.unregister_control_kind(control_id);
            return Err(PlatformError::ControlCreationFailed(format!(
                "TreeView with ID {} was concurrently created for window {window_id:?}",
                control_id.raw()
            )));
        }

        window_data.register_control_hwnd(control_id, hwnd_tv);
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

    let mut tv_item_update = TVITEMEXW {
        mask: TVIF_STATE,
        hItem: h_item_native,
        state: treeview_state_image_mask(new_check_state),
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

    internal_state.with_window_data_write(window_id, |window_data| {
        let tv_state = window_data.get_treeview_state_mut().ok_or_else(|| {
            PlatformError::OperationFailed(format!(
                "No TreeView state exists in window {window_id:?}"
            ))
        })?;
        tv_state.check_states.insert(item_id, new_check_state);
        Ok(())
    })?;

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
        cchTextMax: i32_from_usize_saturating(text_buffer.len()),
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

fn is_user_treeview_selection_action(
    action: windows::Win32::UI::Controls::NM_TREEVIEW_ACTION,
) -> bool {
    action == TVC_BYMOUSE || action == TVC_BYKEYBOARD
}

pub(crate) fn handle_treeview_selection_changed_notification(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    lparam: LPARAM,
    control_id_from_notify: ControlId,
) -> Option<AppEvent> {
    log::trace!(
        "TreeViewHandler: TVN_SELCHANGEDW received for WinID {window_id:?}, ControlID {}",
        control_id_from_notify.raw()
    );

    let nmtv = unsafe { &*(lparam.0 as *const NMTREEVIEWW) };
    if !is_user_treeview_selection_action(nmtv.action) {
        return None;
    }

    let h_item = nmtv.itemNew.hItem;
    if h_item.0 == 0 {
        return None;
    }

    let result = internal_state.with_window_data_read(window_id, |window_data| {
        let tv_state = window_data.get_treeview_state().ok_or_else(|| {
            PlatformError::OperationFailed(format!(
                "TreeView state not found while resolving selection change in WinID {window_id:?}"
            ))
        })?;

        tv_state
            .htreeitem_to_item_id
            .get(&(h_item.0))
            .copied()
            .ok_or_else(|| {
                PlatformError::InvalidHandle(format!(
                    "HTREEITEM {h_item:?} missing in map during selection change"
                ))
            })
    });

    match result {
        Ok(item_id) => Some(AppEvent::TreeViewItemSelectionChanged { window_id, item_id }),
        Err(err) => {
            log::error!(
                "Failed to resolve TreeView selection change in WinID {window_id:?}: {err:?}"
            );
            None
        }
    }
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

    if let Some(item_rect) = treeview_item_rect(hwnd_treeview, htreeitem, false) {
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

fn tree_item_marker_for_display(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    tree_item_id: TreeItemId,
) -> TreeItemMarkerKind {
    let provider_opt = internal_state
        .ui_state_provider()
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|weak_handler| weak_handler.upgrade());

    if let Some(handler_arc) = provider_opt
        && let Ok(handler_guard) = handler_arc.lock()
    {
        return handler_guard.tree_item_marker(window_id, tree_item_id);
    }

    TreeItemMarkerKind::None
}

fn tree_item_marker_color(marker: TreeItemMarkerKind) -> Option<Color> {
    match marker {
        TreeItemMarkerKind::None => None,
        TreeItemMarkerKind::Blue => Some(Color {
            r: 92,
            g: 129,
            b: 176,
        }), // muted blue slate
        TreeItemMarkerKind::Green => Some(Color {
            r: 83,
            g: 171,
            b: 109,
        }), // calm green
        TreeItemMarkerKind::Yellow => Some(Color {
            r: 214,
            g: 158,
            b: 78,
        }), // muted amber / clay
        TreeItemMarkerKind::Red => Some(Color {
            r: 196,
            g: 101,
            b: 76,
        }), // warm terracotta alert
        TreeItemMarkerKind::Purple => Some(Color {
            r: 164,
            g: 130,
            b: 98,
        }), // muted clay neutral
        TreeItemMarkerKind::Gray => Some(Color {
            r: 141,
            g: 130,
            b: 118,
        }), // warm neutral
    }
}

fn rect_seeded_for_treeview_item(h_item_native: HTREEITEM) -> RECT {
    let mut rect = RECT::default();
    unsafe {
        ptr::write_unaligned((&mut rect as *mut RECT).cast::<HTREEITEM>(), h_item_native);
    }
    rect
}

fn treeview_item_rect(
    hwnd_treeview: HWND,
    h_item_native: HTREEITEM,
    text_only: bool,
) -> Option<RECT> {
    let mut rect = rect_seeded_for_treeview_item(h_item_native);
    let success = unsafe {
        SendMessageW(
            hwnd_treeview,
            TVM_GETITEMRECT,
            Some(WPARAM(usize::from(text_only))),
            Some(LPARAM(&mut rect as *mut _ as isize)),
        )
    };
    if success.0 == 0 {
        return None;
    }
    Some(rect)
}

fn treeview_state_image_mask(state: CheckState) -> u32 {
    let image_index = match state {
        // Win32 collapses the state-image lane entirely at index 0. Hidden rows
        // intentionally reuse the unchecked slot and blank that glyph in postpaint
        // so marker/layout alignment stays stable.
        CheckState::Hidden => 1,
        CheckState::Unchecked => 1,
        CheckState::Checked => 2,
    };
    image_index << 12
}

fn tree_item_state_icon_lane_rect(item_rect: RECT, text_rect: RECT) -> Option<RECT> {
    if item_rect.bottom <= item_rect.top
        || text_rect.bottom <= text_rect.top
        || text_rect.left <= item_rect.left
    {
        return None;
    }

    let right = text_rect.left.checked_sub(MARKER_LANE_GAP)?;
    let left = right.checked_sub(STATE_ICON_LANE_WIDTH)?;
    if left < item_rect.left {
        return None;
    }

    Some(RECT {
        left,
        top: item_rect.top,
        right,
        bottom: item_rect.bottom,
    })
}

fn tree_item_marker_rect(item_rect: RECT, text_rect: RECT) -> Option<RECT> {
    if item_rect.bottom <= item_rect.top
        || item_rect.right <= item_rect.left
        || text_rect.bottom <= text_rect.top
        || text_rect.right <= text_rect.left
    {
        return None;
    }

    let lane_rect = tree_item_state_icon_lane_rect(item_rect, text_rect)?;
    let lane_width = lane_rect.right - lane_rect.left;
    let marker_diameter = (text_rect.bottom - text_rect.top - (MARKER_BORDER * 2))
        .clamp(MARKER_MIN_DIAMETER, MARKER_MAX_DIAMETER);
    if lane_width < marker_diameter {
        return None;
    }

    let left = lane_rect.left + (lane_width - marker_diameter) / 2;
    let text_height = text_rect.bottom - text_rect.top;
    let top = text_rect.top + (text_height - marker_diameter) / 2;
    Some(RECT {
        left,
        top,
        right: left + marker_diameter,
        bottom: top + marker_diameter,
    })
}

fn draw_tree_item_marker(
    hdc: HDC,
    hwnd_treeview: HWND,
    h_item_native: HTREEITEM,
    color: Color,
    background_color_ref: windows::Win32::Foundation::COLORREF,
) {
    let Some(item_rect) = treeview_item_rect(hwnd_treeview, h_item_native, false) else {
        return;
    };
    let Some(text_rect) = treeview_item_rect(hwnd_treeview, h_item_native, true) else {
        return;
    };

    let Some(marker_rect) = tree_item_marker_rect(item_rect, text_rect) else {
        return;
    };

    let outer_brush = unsafe { CreateSolidBrush(background_color_ref) };
    if !outer_brush.is_invalid() {
        unsafe {
            let previous_brush = SelectObject(hdc, HGDIOBJ(outer_brush.0));
            let _ = Ellipse(
                hdc,
                marker_rect.left,
                marker_rect.top,
                marker_rect.right,
                marker_rect.bottom,
            );
            SelectObject(hdc, previous_brush);
            let _ = DeleteObject(HGDIOBJ(outer_brush.0));
        }
    }

    let inner_color_ref = styling_handler::color_to_colorref(&color);
    let inner_brush = unsafe { CreateSolidBrush(inner_color_ref) };
    if inner_brush.is_invalid() {
        return;
    }

    unsafe {
        let previous_brush = SelectObject(hdc, HGDIOBJ(inner_brush.0));
        let inner_left = marker_rect.left + MARKER_BORDER;
        let inner_top = marker_rect.top + MARKER_BORDER;
        let inner_right = marker_rect.right - MARKER_BORDER;
        let inner_bottom = marker_rect.bottom - MARKER_BORDER;
        if inner_right > inner_left && inner_bottom > inner_top {
            let _ = Ellipse(hdc, inner_left, inner_top, inner_right, inner_bottom);
        }
        SelectObject(hdc, previous_brush);
        let _ = DeleteObject(HGDIOBJ(inner_brush.0));
    }
}

/*
 * Resolves text and background colors for a TreeView item, accounting for
 * base style, per-item override, and selection state.
 *
 * Returns (text_color, background_color) as optional Color values.
 */
fn resolve_item_colors(
    base_text: Option<&Color>,
    base_bg: Option<&Color>,
    override_text: Option<&Color>,
    override_bg: Option<&Color>,
    is_selected: bool,
    selection_text: Option<&Color>,
    selection_bg: Option<&Color>,
) -> (Option<Color>, Option<Color>) {
    // 1. Start with base style
    let mut text = base_text.cloned();
    let mut bg = base_bg.cloned();

    // 2. Per-item override wins over base
    let has_item_text_override = override_text.is_some();
    if let Some(c) = override_text {
        text = Some(*c);
    }
    if let Some(c) = override_bg {
        bg = Some(*c);
    }

    // 3. Selection: bg always wins; text wins unless per-item override set custom text
    if is_selected {
        if let Some(c) = selection_bg {
            bg = Some(*c);
        }
        if !has_item_text_override && let Some(c) = selection_text {
            text = Some(*c);
        }
    }

    (text, bg)
}

fn treeview_tail_fill_rect(item_draw_rect: RECT, client_rect: RECT) -> Option<RECT> {
    if item_draw_rect.bottom <= item_draw_rect.top || item_draw_rect.right >= client_rect.right {
        return None;
    }

    Some(RECT {
        left: item_draw_rect.right,
        top: item_draw_rect.top,
        right: client_rect.right,
        bottom: item_draw_rect.bottom,
    })
}

fn treeview_selection_accent_rect(item_draw_rect: RECT) -> Option<RECT> {
    if item_draw_rect.bottom <= item_draw_rect.top {
        return None;
    }

    Some(RECT {
        left: 0,
        top: item_draw_rect.top,
        right: SELECTION_ACCENT_WIDTH,
        bottom: item_draw_rect.bottom,
    })
}

fn should_request_postpaint(
    selected_font: Option<HFONT>,
    marker_kind: TreeItemMarkerKind,
    draws_selection_accent: bool,
    hides_state_icon: bool,
) -> bool {
    selected_font.is_some()
        || !matches!(marker_kind, TreeItemMarkerKind::None)
        || draws_selection_accent
        || hides_state_icon
}

fn should_draw_selection_accent(
    was_selected_before_custom_draw: bool,
    has_selection_accent_style: bool,
) -> bool {
    was_selected_before_custom_draw && has_selection_accent_style
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
            let marker_kind = tree_item_marker_for_display(internal_state, window_id, tree_item_id);
            let check_state = internal_state
                .with_window_data_read(window_id, |window_data| {
                    Ok(window_data
                        .get_treeview_state()
                        .and_then(|state| state.check_states.get(&tree_item_id).copied()))
                })
                .unwrap_or(None)
                .unwrap_or(CheckState::Unchecked);

            // Gather base style colors
            let base_style_id = internal_state
                .with_window_data_read(window_id, |window_data| {
                    Ok(window_data.get_style_for_control(control_id_of_treeview))
                })
                .unwrap_or(None);

            let mut selected_font: Option<HFONT> = None;
            let (base_text, base_bg, base_font) = base_style_id
                .and_then(|sid| internal_state.get_parsed_style(sid))
                .map(|s| (s.text_color, s.background_color, s.font_handle))
                .unwrap_or((None, None, None));
            if let Some(f) = base_font {
                selected_font = Some(f);
            }

            // Gather per-item override colors
            let style_override = internal_state
                .with_window_data_read(window_id, |window_data| {
                    Ok(window_data
                        .get_treeview_state()
                        .and_then(|state| state.style_override_for(&tree_item_id)))
                })
                .unwrap_or(None);

            let (override_text, override_bg, override_font) = style_override
                .and_then(|sid| internal_state.get_parsed_style(sid))
                .map(|s| (s.text_color, s.background_color, s.font_handle))
                .unwrap_or((None, None, None));
            if let Some(f) = override_font {
                selected_font = Some(f);
            }

            // Gather selection style colors
            let selection_style = internal_state.get_parsed_style(StyleId::TreeViewSelectedRow);
            let (selection_text, selection_bg) = selection_style
                .as_ref()
                .map(|s| (s.text_color, s.background_color))
                .unwrap_or((None, None));

            // Detect selection
            let is_selected = (nmtvcd.nmcd.uItemState.0 & CDIS_SELECTED.0) != 0;

            // Resolve final colors
            let (resolved_text, resolved_bg) = resolve_item_colors(
                base_text.as_ref(),
                base_bg.as_ref(),
                override_text.as_ref(),
                override_bg.as_ref(),
                is_selected,
                selection_text.as_ref(),
                selection_bg.as_ref(),
            );

            // Apply colors and track whether any color was modified
            let mut color_modified = false;
            if let Some(color) = resolved_text.as_ref() {
                nmtvcd.clrText = styling_handler::color_to_colorref(color);
                color_modified = true;
            }
            if let Some(bg) = resolved_bg.as_ref() {
                nmtvcd.clrTextBk = styling_handler::color_to_colorref(bg);
                color_modified = true;
            }

            // If selected and selection style is defined: suppress native highlight
            let has_selection_style = selection_style.is_some();
            if is_selected && has_selection_style {
                nmtvcd.nmcd.uItemState.0 &= !CDIS_SELECTED.0;
                nmtvcd.nmcd.uItemState.0 &= !CDIS_FOCUS.0;
            }

            // Fill the strip from the draw rect to the client edge so that
            // the resolved background extends to the full row width. This is
            // needed for both selected rows (highlight) and deselected rows
            // (clearing previous highlight artifacts).
            if let Some(bg) = resolved_bg.as_ref() {
                let hwnd_treeview = nmtvcd.nmcd.hdr.hwndFrom;
                let hdc = nmtvcd.nmcd.hdc;
                let mut client_rect = RECT::default();
                let _ = unsafe { GetClientRect(hwnd_treeview, &mut client_rect) };
                if let Some(fill_rect) = treeview_tail_fill_rect(nmtvcd.nmcd.rc, client_rect) {
                    let bg_brush =
                        unsafe { CreateSolidBrush(styling_handler::color_to_colorref(bg)) };
                    if !bg_brush.is_invalid() {
                        unsafe {
                            let _ = FillRect(hdc, &fill_rect, bg_brush);
                            let _ = DeleteObject(HGDIOBJ(bg_brush.0));
                        }
                    }
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

            let selection_accent_color = internal_state
                .get_parsed_style(StyleId::TreeViewSelectionAccent)
                .and_then(|style| style.background_color);
            let draws_selection_accent =
                should_draw_selection_accent(is_selected, selection_accent_color.is_some());
            let hides_state_icon = check_state == CheckState::Hidden;

            let mut result: isize = CDRF_DODEFAULT as isize;
            if let Some(font_handle) = selected_font {
                unsafe {
                    SelectObject(nmtvcd.nmcd.hdc, HGDIOBJ(font_handle.0));
                }
                result |= CDRF_NEWFONT as isize;
            } else if color_modified || (is_selected && has_selection_style) {
                // Colors were modified but no font — still need CDRF_NEWFONT for Windows to apply
                // the changed clrText/clrTextBk values.
                result |= CDRF_NEWFONT as isize;
            }
            if should_request_postpaint(
                selected_font,
                marker_kind,
                draws_selection_accent,
                hides_state_icon,
            ) {
                result |= CDRF_NOTIFYPOSTPAINT as isize;
            }

            return LRESULT(result);
        }
        CDDS_ITEMPOSTPAINT => {
            let hdc = nmtvcd.nmcd.hdc;
            let hwnd_treeview = nmtvcd.nmcd.hdr.hwndFrom;
            let tree_item_id = TreeItemId(nmtvcd.nmcd.lItemlParam.0 as u64);
            let marker_kind = tree_item_marker_for_display(internal_state, window_id, tree_item_id);
            let check_state = internal_state
                .with_window_data_read(window_id, |window_data| {
                    Ok(window_data
                        .get_treeview_state()
                        .and_then(|state| state.check_states.get(&tree_item_id).copied()))
                })
                .unwrap_or(None)
                .unwrap_or(CheckState::Unchecked);

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
            if check_state == CheckState::Hidden {
                let h_item_native = HTREEITEM(nmtvcd.nmcd.dwItemSpec as isize);
                if let Some(item_rect) = treeview_item_rect(hwnd_treeview, h_item_native, false)
                    && let Some(text_rect) = treeview_item_rect(hwnd_treeview, h_item_native, true)
                    && let Some(state_icon_rect) =
                        tree_item_state_icon_lane_rect(item_rect, text_rect)
                {
                    let hidden_lane_brush = unsafe { CreateSolidBrush(nmtvcd.clrTextBk) };
                    if !hidden_lane_brush.is_invalid() {
                        unsafe {
                            let _ = FillRect(hdc, &state_icon_rect, hidden_lane_brush);
                            let _ = DeleteObject(HGDIOBJ(hidden_lane_brush.0));
                        }
                    }
                }
            }
            if let Some(color) = tree_item_marker_color(marker_kind) {
                let h_item_native = HTREEITEM(nmtvcd.nmcd.dwItemSpec as isize);
                draw_tree_item_marker(hdc, hwnd_treeview, h_item_native, color, nmtvcd.clrTextBk);
            }

            // Draw selection accent bar if this item is selected and accent style is defined.
            let h_item_native = HTREEITEM(nmtvcd.nmcd.dwItemSpec as isize);
            let caret_lresult = unsafe {
                SendMessageW(
                    hwnd_treeview,
                    TVM_GETNEXTITEM,
                    Some(WPARAM(TVGN_CARET as usize)),
                    Some(LPARAM(0)),
                )
            };
            let caret_item = HTREEITEM(caret_lresult.0 as isize);
            if should_draw_selection_accent(
                caret_item == h_item_native,
                internal_state
                    .get_parsed_style(StyleId::TreeViewSelectionAccent)
                    .and_then(|style| style.background_color)
                    .is_some(),
            ) && let Some(accent_style) =
                internal_state.get_parsed_style(StyleId::TreeViewSelectionAccent)
                && let Some(accent_color) = accent_style.background_color.as_ref()
                && let Some(accent_rect) = treeview_selection_accent_rect(nmtvcd.nmcd.rc)
            {
                let accent_brush =
                    unsafe { CreateSolidBrush(styling_handler::color_to_colorref(accent_color)) };
                if !accent_brush.is_invalid() {
                    unsafe {
                        let _ = FillRect(hdc, &accent_rect, accent_brush);
                        let _ = DeleteObject(HGDIOBJ(accent_brush.0));
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

        let stored_check_state = tv_state
            .check_states
            .get(&app_item_id)
            .copied()
            .unwrap_or(CheckState::Unchecked);
        // This map reflects the last host-confirmed state. For normal checkbox rows
        // it can be briefly stale between the user's click and the host's follow-up
        // UpdateTreeItemVisualState command, which is acceptable because hidden-row
        // suppression is the only behavior that depends on it inside this handler.
        if stored_check_state == CheckState::Hidden {
            let mut tv_item_restore = TVITEMEXW {
                mask: TVIF_STATE,
                hItem: h_item_clicked,
                state: treeview_state_image_mask(CheckState::Hidden),
                stateMask: TVIS_STATEIMAGEMASK.0,
                ..Default::default()
            };
            let _ = unsafe {
                SendMessageW(
                    hwnd_treeview,
                    TVM_SETITEMW,
                    Some(WPARAM(0)),
                    Some(LPARAM(&mut tv_item_restore as *mut _ as isize)),
                )
            };
            return Ok(None);
        }

        let state_image_idx = (tv_item_get.state & TVIS_STATEIMAGEMASK.0) >> 12;
        let new_check_state = if state_image_idx == 2 {
            CheckState::Checked
        } else {
            CheckState::Unchecked
        };

        Ok(Some(AppEvent::TreeViewItemToggledByUser {
            window_id,
            item_id: app_item_id,
            new_state: new_check_state,
        }))
    });

    match result {
        Ok(event) => event,
        Err(e) => {
            log::error!("Failed to handle checkbox click for HTREEITEM {h_item_clicked:?}: {e:?}");
            None
        }
    }
}

/*
 * Handles general NM_CLICK notifications for a TreeView.
 * This function's primary purpose is to detect clicks on a TreeView item's state
 * icon (checkbox) and post a custom message for deferred processing. Selection
 * changes are handled through TVN_SELCHANGEDW so mouse and keyboard navigation
 * share the same path.
 */
pub(crate) fn handle_nm_click(
    _internal_state: &Arc<Win32ApiInternalState>,
    parent_hwnd: HWND,
    _window_id: WindowId,
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

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b }
    }

    fn rect_width(rect: RECT) -> i32 {
        rect.right - rect.left
    }

    fn rect_height(rect: RECT) -> i32 {
        rect.bottom - rect.top
    }

    #[test]
    fn resolve_item_colors_no_styles() {
        let (text, bg) = resolve_item_colors(None, None, None, None, false, None, None);
        assert_eq!(text, None);
        assert_eq!(bg, None);
    }

    #[test]
    fn resolve_item_colors_base_only() {
        let base_text = c(200, 200, 200);
        let base_bg = c(30, 30, 30);
        let (text, bg) = resolve_item_colors(
            Some(&base_text),
            Some(&base_bg),
            None,
            None,
            false,
            None,
            None,
        );
        assert_eq!(text, Some(base_text));
        assert_eq!(bg, Some(base_bg));
    }

    #[test]
    fn resolve_item_colors_base_selected_no_selection_style() {
        let base_text = c(200, 200, 200);
        let base_bg = c(30, 30, 30);
        let (text, bg) = resolve_item_colors(
            Some(&base_text),
            Some(&base_bg),
            None,
            None,
            true,
            None,
            None,
        );
        // No selection style — colors unchanged
        assert_eq!(text, Some(base_text));
        assert_eq!(bg, Some(base_bg));
    }

    #[test]
    fn resolve_item_colors_base_selected_with_selection_style() {
        let base_text = c(200, 200, 200);
        let base_bg = c(30, 30, 30);
        let sel_text = c(255, 255, 255);
        let sel_bg = c(55, 62, 71);
        let (text, bg) = resolve_item_colors(
            Some(&base_text),
            Some(&base_bg),
            None,
            None,
            true,
            Some(&sel_text),
            Some(&sel_bg),
        );
        assert_eq!(text, Some(sel_text));
        assert_eq!(bg, Some(sel_bg));
    }

    #[test]
    fn resolve_item_colors_per_item_override_selected_preserves_override_text() {
        let base_text = c(200, 200, 200);
        let base_bg = c(30, 30, 30);
        let override_text = c(96, 101, 107); // muted/disabled
        let sel_text = c(255, 255, 255);
        let sel_bg = c(55, 62, 71);
        let (text, bg) = resolve_item_colors(
            Some(&base_text),
            Some(&base_bg),
            Some(&override_text),
            None,
            true,
            Some(&sel_text),
            Some(&sel_bg),
        );
        // Per-item text override is preserved even when selected
        assert_eq!(text, Some(override_text));
        // But selection bg still applies
        assert_eq!(bg, Some(sel_bg));
    }

    #[test]
    fn resolve_item_colors_per_item_override_not_selected() {
        let base_text = c(200, 200, 200);
        let base_bg = c(30, 30, 30);
        let override_text = c(96, 101, 107);
        let override_bg = c(40, 40, 40);
        let (text, bg) = resolve_item_colors(
            Some(&base_text),
            Some(&base_bg),
            Some(&override_text),
            Some(&override_bg),
            false,
            None,
            None,
        );
        assert_eq!(text, Some(override_text));
        assert_eq!(bg, Some(override_bg));
    }

    #[test]
    fn treeview_tail_fill_rect_uses_draw_rect_edge() {
        let item_rect = RECT {
            left: 24,
            top: 10,
            right: 160,
            bottom: 30,
        };
        let client_rect = RECT {
            left: 0,
            top: 0,
            right: 320,
            bottom: 400,
        };

        let fill_rect = treeview_tail_fill_rect(item_rect, client_rect);

        let fill_rect =
            fill_rect.expect("tail fill rect should exist when draw rect ends before client");
        assert_eq!(fill_rect.left, item_rect.right);
        assert_eq!(fill_rect.top, item_rect.top);
        assert_eq!(fill_rect.right, client_rect.right);
        assert_eq!(fill_rect.bottom, item_rect.bottom);
    }

    #[test]
    fn tree_item_marker_rect_anchors_before_text_lane() {
        let item_rect = RECT {
            left: 0,
            top: 10,
            right: 200,
            bottom: 30,
        };
        let text_rect = RECT {
            left: 40,
            top: 10,
            right: 160,
            bottom: 30,
        };

        let lane_rect = tree_item_state_icon_lane_rect(item_rect, text_rect)
            .expect("state icon lane should exist");
        let marker_rect =
            tree_item_marker_rect(item_rect, text_rect).expect("marker rect should exist");

        assert_eq!(rect_width(marker_rect), rect_height(marker_rect));
        assert!((MARKER_MIN_DIAMETER..=MARKER_MAX_DIAMETER).contains(&rect_width(marker_rect)));
        assert!(marker_rect.left >= lane_rect.left);
        assert!(marker_rect.right <= lane_rect.right);
        assert!(marker_rect.right <= text_rect.left);
        assert!(marker_rect.top >= text_rect.top);
        assert!(marker_rect.bottom <= text_rect.bottom);
        assert_eq!(
            (marker_rect.left - lane_rect.left) - (lane_rect.right - marker_rect.right),
            0,
            "marker should stay horizontally centered in the reserved lane"
        );
    }

    #[test]
    fn tree_item_marker_rect_returns_none_when_lane_is_too_small() {
        let item_rect = RECT {
            left: 0,
            top: 10,
            right: 200,
            bottom: 30,
        };
        let text_rect = RECT {
            left: 14,
            top: 10,
            right: 160,
            bottom: 30,
        };

        assert_eq!(tree_item_marker_rect(item_rect, text_rect), None);
    }

    #[test]
    fn treeview_state_image_mask_preserves_hidden_lane() {
        let hidden = treeview_state_image_mask(CheckState::Hidden);
        let unchecked = treeview_state_image_mask(CheckState::Unchecked);
        let checked = treeview_state_image_mask(CheckState::Checked);

        assert_eq!(hidden, unchecked);
        assert_ne!(checked, hidden);
        assert_eq!(checked >> 12, 2);
        assert_eq!(hidden >> 12, 1);
    }

    #[test]
    fn tree_item_state_icon_lane_rect_reserves_space_before_text() {
        let item_rect = RECT {
            left: 0,
            top: 10,
            right: 200,
            bottom: 30,
        };
        let text_rect = RECT {
            left: 40,
            top: 12,
            right: 160,
            bottom: 28,
        };

        let lane_rect = tree_item_state_icon_lane_rect(item_rect, text_rect)
            .expect("state icon lane should exist");

        assert_eq!(lane_rect.top, item_rect.top);
        assert_eq!(lane_rect.bottom, item_rect.bottom);
        assert_eq!(rect_width(lane_rect), STATE_ICON_LANE_WIDTH);
        assert_eq!(text_rect.left - lane_rect.right, MARKER_LANE_GAP);
        assert!(lane_rect.left >= item_rect.left);
        assert!(lane_rect.right <= text_rect.left);
    }

    #[test]
    fn tree_item_state_icon_lane_rect_returns_none_when_text_starts_before_item() {
        let item_rect = RECT {
            left: 30,
            top: 10,
            right: 200,
            bottom: 30,
        };
        let text_rect = RECT {
            left: 30,
            top: 12,
            right: 160,
            bottom: 28,
        };

        assert_eq!(tree_item_state_icon_lane_rect(item_rect, text_rect), None);
    }

    #[test]
    fn tree_item_marker_color_uses_warm_palette() {
        let red = tree_item_marker_color(TreeItemMarkerKind::Red).expect("red marker color");
        let yellow =
            tree_item_marker_color(TreeItemMarkerKind::Yellow).expect("yellow marker color");
        let gray = tree_item_marker_color(TreeItemMarkerKind::Gray).expect("gray marker color");

        assert!(
            red.r > red.g && red.g > red.b,
            "red marker should skew warm"
        );
        assert!(
            yellow.r >= yellow.g && yellow.g > yellow.b,
            "yellow marker should skew warm"
        );
        assert!(
            (gray.r as i16 - gray.g as i16).abs() <= 16
                && (gray.g as i16 - gray.b as i16).abs() <= 16,
            "gray marker should stay near-neutral"
        );
        assert_ne!(red, yellow);
        assert_ne!(yellow, gray);
        assert_ne!(red, gray);
    }

    #[test]
    fn should_request_postpaint_stays_false_for_color_only_rows() {
        assert!(!should_request_postpaint(
            None,
            TreeItemMarkerKind::None,
            false,
            false,
        ));
    }

    #[test]
    fn should_request_postpaint_when_selection_accent_is_drawn() {
        assert!(should_request_postpaint(
            None,
            TreeItemMarkerKind::None,
            true,
            false,
        ));
    }

    #[test]
    fn should_request_postpaint_when_marker_is_drawn() {
        assert!(should_request_postpaint(
            None,
            TreeItemMarkerKind::Blue,
            false,
            false,
        ));
    }

    #[test]
    fn should_request_postpaint_when_hidden_state_icon_is_erased() {
        assert!(should_request_postpaint(
            None,
            TreeItemMarkerKind::None,
            false,
            true,
        ));
    }

    #[test]
    fn should_draw_selection_accent_when_item_was_selected_and_style_exists() {
        assert!(should_draw_selection_accent(true, true));
    }

    #[test]
    fn should_not_draw_selection_accent_when_selected_flag_was_not_present() {
        assert!(!should_draw_selection_accent(false, true));
    }

    #[test]
    fn user_treeview_selection_action_accepts_mouse_and_keyboard() {
        assert!(is_user_treeview_selection_action(TVC_BYMOUSE));
        assert!(is_user_treeview_selection_action(TVC_BYKEYBOARD));
    }
}
