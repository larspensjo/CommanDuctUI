/*
 * Encapsulates Win32-specific operations for button controls.
 * Provides creation of push buttons and translation of button click
 * notifications into platform-agnostic `AppEvent`s.
 */

use crate::app::Win32ApiInternalState;
use crate::controls::styling_handler::{color_to_colorref, colorref_to_color};
use crate::error::{PlatformError, Result as PlatformResult};
use crate::ffi_safety;
use crate::styling::{Color, StyleId, TextAlignment};
use crate::types::{AppEvent, ControlId, WindowId};
use crate::window_common::ControlKind;

use std::sync::Arc;
use windows::Win32::{
    Foundation::{COLORREF, HANDLE, HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{
        COLOR_BTNFACE, COLOR_BTNTEXT, CreatePen, CreateSolidBrush, DT_CENTER, DT_END_ELLIPSIS,
        DT_LEFT, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawFocusRect, DrawTextW, FillRect,
        GetSysColor, GetTextExtentPoint32W, HDC, HGDIOBJ, InflateRect, InvalidateRect, LineTo,
        MoveToEx, OPAQUE, PS_SOLID, SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
    },
    UI::Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_HOTLIGHT, ODS_SELECTED},
    UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent},
    UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
    UI::WindowsAndMessaging::{
        BS_PUSHBUTTON, CreateWindowExW, DestroyWindow, GetDlgCtrlID, GetWindowTextLengthW,
        GetWindowTextW, HMENU, RemovePropW, SetPropW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_MOUSEMOVE,
        WM_NCDESTROY, WS_CHILD, WS_VISIBLE,
    },
};
use windows::core::{HSTRING, PCWSTR};

const WC_BUTTON: PCWSTR = windows::core::w!("BUTTON");
const BUTTON_HOVER_PROP: PCWSTR = windows::core::w!("CommanDuctUI.ButtonHover");
const BUTTON_HOVER_SUBCLASS_ID: usize = 1;
// WM_MOUSELEAVE is not exported by windows-rs in this crate version.
const WM_MOUSELEAVE: u32 = 0x02A3;
const DISABLED_BG_TARGET: Color = Color {
    r: 0x1E,
    g: 0x1E,
    b: 0x1C,
};
const DISABLED_TEXT_COLOR: Color = Color {
    r: 0x87,
    g: 0x86,
    b: 0x7F,
};

/*
 * Creates a native push button and registers the resulting HWND in the
 * window's `NativeWindowData`. Fails if the window or control ID are
 * invalid or already in use. This function uses a read-create-write pattern
 * to minimize lock contention on the global window map.
 *
 * First, it acquires a read lock to verify that the control doesn't already
 * exist and to get the parent HWND. Then, it creates the native button
 * control without holding any locks. Finally, it acquires a write lock briefly
 * to register the new control, checking for race conditions.
 * [CDU-Control-ButtonV1][CDU-IdempotentCommandsV1] Push buttons are created exactly once per logical ID and surface clicks via AppEvents.
 */
pub(crate) fn handle_create_button_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
    text: String,
) -> PlatformResult<()> {
    log::debug!(
        "ButtonHandler: handle_create_button_command for WinID {window_id:?}, ParentID {:?}, ControlID {}, Text: '{text}'",
        parent_control_id.as_ref().map(|id| id.raw()),
        control_id.raw()
    );

    // Phase 1: Read-only pre-checks.
    // Get the parent HWND for creation while holding only a read lock.
    let hwnd_parent_for_creation =
        internal_state.with_window_data_read(window_id, |window_data| {
            if window_data.has_control(control_id) {
                log::warn!(
                    "ButtonHandler: Button with ID {} already exists for window {window_id:?}.",
                    control_id.raw()
                );
                return Err(PlatformError::OperationFailed(format!(
                    "Button with ID {} already exists for window {window_id:?}",
                    control_id.raw()
                )));
            }

            let hwnd_parent = match parent_control_id {
                Some(id) => window_data.get_control_hwnd(id).ok_or_else(|| {
                    log::warn!(
                        "ButtonHandler: Parent control with ID {} not found for CreateButton in WinID {window_id:?}",
                        id.raw()
                    );
                    PlatformError::InvalidHandle(format!(
                        "Parent control with ID {} not found for CreateButton in WinID {window_id:?}",
                        id.raw()
                    ))
                })?,
                None => window_data.get_hwnd(),
            };

            if hwnd_parent.is_invalid() {
                log::error!(
                    "ButtonHandler: Parent HWND for CreateButton is invalid (WinID: {window_id:?}, ParentControlID: {:?})",
                    parent_control_id.as_ref().map(|id| id.raw())
                );
                return Err(PlatformError::InvalidHandle(format!(
                    "ButtonHandler: Parent HWND for CreateButton is invalid (WinID: {window_id:?}, ParentControlID: {:?})",
                    parent_control_id.as_ref().map(|id| id.raw())
                )));
            }
            Ok(hwnd_parent)
        })?;

    internal_state.with_window_data_write(window_id, |window_data| {
        if window_data.has_control(control_id) {
            log::warn!(
                "ButtonHandler: Button with ID {} already exists for window {window_id:?}.",
                control_id.raw()
            );
            return Err(PlatformError::OperationFailed(format!(
                "Button with ID {} already exists for window {window_id:?}",
                control_id.raw()
            )));
        }
        window_data.register_control_kind(control_id, ControlKind::Button);
        Ok(())
    })?;

    // Phase 2: Create the native control without holding any locks.
    let h_instance = internal_state.h_instance();
    let hwnd_button = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WC_BUTTON,
            &HSTRING::from(text.as_str()),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_PUSHBUTTON as u32),
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

    install_button_hover_subclass(hwnd_button);

    // Phase 3: Acquire a write lock only to register the new HWND.
    internal_state.with_window_data_write(window_id, |window_data| {
        // Re-check for a race condition where another thread created the control
        // while we were not holding a lock.
        if window_data.has_control(control_id) {
            log::warn!(
                "ButtonHandler: Control ID {} was created concurrently for window {window_id:?}. Destroying new HWND.",
                control_id.raw()
            );
            unsafe {
                // Safely ignore error if window is already gone.
                DestroyWindow(hwnd_button).ok();
            }
            return Err(PlatformError::OperationFailed(format!(
                "Button with ID {} was created concurrently for window {window_id:?}",
                control_id.raw()
            )));
        }

        window_data.register_control_hwnd(control_id, hwnd_button);
        log::debug!(
            "ButtonHandler: Created button '{text}' (ID {}) for window {window_id:?} with HWND {hwnd_button:?}",
            control_id.raw()
        );
        Ok(())
    })
}

/*
 * Translates a BN_CLICKED notification into an `AppEvent::ButtonClicked`.
 */
pub(crate) fn handle_bn_clicked(
    window_id: WindowId,
    control_id: ControlId,
    hwnd_control: HWND,
) -> AppEvent {
    // [CDU-Control-ButtonV1] Button notifications re-enter the command pipeline as type-safe events carrying the original ControlId.
    log::debug!(
        "ButtonHandler: BN_CLICKED for ID {} (HWND {hwnd_control:?}) in WinID {window_id:?}",
        control_id.raw()
    );
    AppEvent::ButtonClicked {
        window_id,
        control_id,
    }
}

/*
 * Handles WM_DRAWITEM for owner-drawn buttons.
 * Renders buttons with custom background and text colors from applied styles.
 * Supports disabled state, pressed state, and focus rectangle.
 */
pub(crate) fn handle_wm_drawitem(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    draw_item_struct: *const DRAWITEMSTRUCT,
) -> Option<LRESULT> {
    unsafe {
        if draw_item_struct.is_null() {
            return None;
        }
        let dis = &*draw_item_struct;
        let control_id = ControlId::new(dis.CtlID as i32);

        // Get applied style with fallback to system colors
        let style_id = internal_state
            .with_window_data_read(window_id, |window_data| {
                Ok(window_data.get_style_for_control(control_id))
            })
            .ok()
            .flatten();
        let style = style_id.and_then(|sid| internal_state.get_parsed_style(sid));

        // Resolve colors: style values or system defaults as fallback
        let base_bg = style
            .as_ref()
            .and_then(|s| s.background_color)
            .unwrap_or_else(|| colorref_to_color(COLORREF(GetSysColor(COLOR_BTNFACE))));
        let base_fg = style
            .as_ref()
            .and_then(|s| s.text_color)
            .unwrap_or_else(|| colorref_to_color(COLORREF(GetSysColor(COLOR_BTNTEXT))));

        // Determine final colors based on button state
        let is_disabled = (dis.itemState.0 & ODS_DISABLED.0) != 0;
        let is_pressed = (dis.itemState.0 & ODS_SELECTED.0) != 0;

        let (bg_color, text_color) = if is_disabled {
            resolve_disabled_button_colors(base_bg, base_fg)
        } else if is_pressed {
            // Pressed: darken background by 20%
            let pressed_bg = Color {
                r: (base_bg.r as u32 * 80 / 100) as u8,
                g: (base_bg.g as u32 * 80 / 100) as u8,
                b: (base_bg.b as u32 * 80 / 100) as u8,
            };
            (pressed_bg, base_fg)
        } else {
            (base_bg, base_fg)
        };

        // Fill background
        let brush = CreateSolidBrush(color_to_colorref(&bg_color));
        FillRect(dis.hDC, &dis.rcItem, brush);
        let _ = DeleteObject(brush.into());

        // Get button text (dynamic length, no hardcoded buffer)
        let text_len = GetWindowTextLengthW(dis.hwndItem);
        let mut text_buf = vec![0u16; (text_len + 1) as usize];
        GetWindowTextW(dis.hwndItem, &mut text_buf);

        // Draw text
        SetTextColor(dis.hDC, color_to_colorref(&text_color));
        SetBkMode(dis.hDC, TRANSPARENT);

        // Apply font if available, saving old font for restoration
        let old_font = style
            .as_ref()
            .and_then(|s| s.font_handle)
            .map(|font| SelectObject(dis.hDC, HGDIOBJ(font.0)));

        let mut rect = dis.rcItem;
        let text_flags = match style
            .as_ref()
            .map(|parsed_style| parsed_style.text_alignment)
        {
            Some(TextAlignment::Left) => {
                let _ = InflateRect(&mut rect, -8, 0);
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS
            }
            _ => DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        };
        DrawTextW(
            dis.hDC,
            &mut text_buf[..text_len as usize],
            &mut rect,
            text_flags,
        );

        if should_underline_button(style_id, dis.itemState.0, is_button_hovered(dis.hwndItem)) {
            draw_text_underline(dis.hDC, &text_buf[..text_len as usize], rect, text_color);
        }

        // Restore original font to avoid leaking GDI selection state
        if let Some(prev_font) = old_font {
            SelectObject(dis.hDC, prev_font);
        }

        // Draw focus rectangle (inset by 3px; scale by DPI in future)
        if (dis.itemState.0 & ODS_FOCUS.0) != 0 {
            let mut focus_rect = dis.rcItem;
            let _ = InflateRect(&mut focus_rect, -3, -3);
            let _ = DrawFocusRect(dis.hDC, &focus_rect);
        }

        Some(LRESULT(1)) // TRUE - we handled it
    }
}

pub(crate) fn handle_wm_ctlcolorbtn(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    hdc_button: HDC,
    hwnd_button: HWND,
) -> Option<LRESULT> {
    let control_id_raw = unsafe { GetDlgCtrlID(hwnd_button) };
    if control_id_raw == 0 {
        return None;
    }
    let control_id = ControlId::new(control_id_raw);

    let result: PlatformResult<Option<LRESULT>> =
        internal_state.with_window_data_read(window_id, |window_data| {
            if let Some(style_id) = window_data.get_style_for_control(control_id)
                && let Some(style) = internal_state.get_parsed_style(style_id)
            {
                if let Some(color) = &style.text_color {
                    unsafe { SetTextColor(hdc_button, color_to_colorref(color)) };
                }
                if let Some(color) = &style.background_color {
                    unsafe {
                        SetBkColor(hdc_button, color_to_colorref(color));
                        SetBkMode(hdc_button, OPAQUE);
                    }
                }
                if let Some(brush) = style.background_brush {
                    return Ok(Some(LRESULT(brush.0 as isize)));
                }
            }
            Ok(None)
        });

    result.ok().flatten()
}

unsafe extern "system" fn button_hover_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    ffi_safety::catch_unwind_ffi(
        "button_hover_subclass_proc",
        || unsafe {
            match msg {
                WM_MOUSEMOVE if !is_button_hovered(hwnd) => {
                    let _ = SetPropW(
                        hwnd,
                        BUTTON_HOVER_PROP,
                        Some(HANDLE(std::ptr::dangling_mut())),
                    );
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                WM_MOUSELEAVE if is_button_hovered(hwnd) => {
                    let _ = RemovePropW(hwnd, BUTTON_HOVER_PROP);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                WM_NCDESTROY => {
                    let _ = RemovePropW(hwnd, BUTTON_HOVER_PROP);
                    let _ = RemoveWindowSubclass(
                        hwnd,
                        Some(button_hover_subclass_proc),
                        BUTTON_HOVER_SUBCLASS_ID,
                    );
                }
                _ => {}
            }

            DefSubclassProc(hwnd, msg, wparam, lparam)
        },
        || unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    )
}

fn install_button_hover_subclass(hwnd: HWND) {
    unsafe {
        if !SetWindowSubclass(
            hwnd,
            Some(button_hover_subclass_proc),
            BUTTON_HOVER_SUBCLASS_ID,
            0,
        )
        .as_bool()
        {
            log::warn!("Failed to install button hover subclass for hwnd {hwnd:?}.");
        }
    }
}

fn is_button_hovered(hwnd: HWND) -> bool {
    unsafe {
        !windows::Win32::UI::WindowsAndMessaging::GetPropW(hwnd, BUTTON_HOVER_PROP)
            .0
            .is_null()
    }
}

fn should_underline_button(style_id: Option<StyleId>, item_state: u32, is_hovered: bool) -> bool {
    matches!(style_id, Some(StyleId::LinkButton))
        && (is_hovered || (item_state & (ODS_FOCUS.0 | ODS_HOTLIGHT.0)) != 0)
}

fn draw_text_underline(
    hdc: HDC,
    text: &[u16],
    text_rect: windows::Win32::Foundation::RECT,
    color: Color,
) {
    if text.is_empty() || text_rect.right <= text_rect.left || text_rect.bottom <= text_rect.top {
        return;
    }

    let mut size = windows::Win32::Foundation::SIZE::default();
    unsafe {
        let _ = GetTextExtentPoint32W(hdc, text, &mut size);
    }

    let underline_width = (text_rect.right - text_rect.left).min(size.cx).max(0);
    if underline_width <= 0 {
        return;
    }

    let text_height = size.cy.max(1);
    let vertical_padding = ((text_rect.bottom - text_rect.top - text_height).max(0)) / 2;
    let underline_y = (text_rect.top + vertical_padding + text_height).min(text_rect.bottom - 1);

    unsafe {
        let pen = CreatePen(PS_SOLID, 1, color_to_colorref(&color));
        if pen.is_invalid() {
            return;
        }

        let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
        let _ = MoveToEx(hdc, text_rect.left, underline_y, None);
        let _ = LineTo(hdc, text_rect.left + underline_width, underline_y);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(pen.into());
    }
}

fn resolve_disabled_button_colors(base_bg: Color, base_fg: Color) -> (Color, Color) {
    (
        blend_color(base_bg, DISABLED_BG_TARGET, 1, 2),
        blend_color(base_fg, DISABLED_TEXT_COLOR, 1, 2),
    )
}

fn blend_color(a: Color, b: Color, a_weight: u16, b_weight: u16) -> Color {
    let total = u32::from(a_weight) + u32::from(b_weight);
    Color {
        r: ((u32::from(a.r) * u32::from(a_weight) + u32::from(b.r) * u32::from(b_weight)) / total)
            as u8,
        g: ((u32::from(a.g) * u32::from(a_weight) + u32::from(b.g) * u32::from(b_weight)) / total)
            as u8,
        b: ((u32::from(a.b) * u32::from(a_weight) + u32::from(b.b) * u32::from(b_weight)) / total)
            as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // [CDU-Control-ButtonV1] BN_CLICKED notifications emit the expected `AppEvent::ButtonClicked`.
    fn bn_clicked_translates_to_app_event() {
        let event = handle_bn_clicked(WindowId(9), ControlId::new(5), HWND::default());
        match event {
            AppEvent::ButtonClicked {
                window_id,
                control_id,
            } => {
                assert_eq!(window_id, WindowId(9));
                assert_eq!(control_id, ControlId::new(5));
            }
            other => panic!("Unexpected event: {other:?}"),
        }
    }

    #[test]
    fn disabled_button_colors_are_muted_toward_theme_neutrals() {
        let (bg, text) = resolve_disabled_button_colors(
            Color {
                r: 0xB5,
                g: 0x33,
                b: 0x33,
            },
            Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            },
        );
        assert_eq!(
            bg,
            Color {
                r: 0x50,
                g: 0x25,
                b: 0x23,
            }
        );
        assert_eq!(
            text,
            Color {
                r: 0xAD,
                g: 0xAC,
                b: 0xA6,
            }
        );
    }

    #[test]
    fn blend_color_biases_toward_second_color_by_weight() {
        let blended = blend_color(
            Color {
                r: 0x10,
                g: 0x20,
                b: 0x30,
            },
            Color {
                r: 0x40,
                g: 0x50,
                b: 0x60,
            },
            1,
            3,
        );
        assert_eq!(
            blended,
            Color {
                r: 0x34,
                g: 0x44,
                b: 0x54,
            }
        );
    }

    #[test]
    fn link_buttons_underline_when_focused() {
        assert!(should_underline_button(
            Some(StyleId::LinkButton),
            ODS_FOCUS.0,
            false
        ));
    }

    #[test]
    fn link_buttons_underline_when_hotlighted() {
        assert!(should_underline_button(
            Some(StyleId::LinkButton),
            ODS_HOTLIGHT.0,
            false
        ));
    }

    #[test]
    fn link_buttons_underline_when_hovered() {
        assert!(should_underline_button(Some(StyleId::LinkButton), 0, true));
    }

    #[test]
    fn non_link_buttons_do_not_underline_on_focus() {
        assert!(!should_underline_button(
            Some(StyleId::SecondaryButton),
            ODS_FOCUS.0,
            true
        ));
    }
}
