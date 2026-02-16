use crate::window_common::ControlKind;
use log::{debug, warn};
use windows::Win32::UI::WindowsAndMessaging::{
    WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaintRoute {
    LabelStatic,
    Edit,
    ComboListBox,
    Default,
}

pub(crate) fn resolve_paint_route(kind: ControlKind, msg: u32) -> PaintRoute {
    match (kind, msg) {
        (ControlKind::Edit, WM_CTLCOLORSTATIC) => {
            debug!("[Paint] ControlKind::Edit routed via WM_CTLCOLORSTATIC to edit styling");
            PaintRoute::Edit
        }
        (ControlKind::Edit, WM_CTLCOLOREDIT) => PaintRoute::Edit,
        (ControlKind::ComboBox, WM_CTLCOLORLISTBOX) => {
            debug!("[Paint] ControlKind::ComboBox routed WM_CTLCOLORLISTBOX to combo listbox styling");
            PaintRoute::ComboListBox
        }
        (ControlKind::Static, WM_CTLCOLORSTATIC) => PaintRoute::LabelStatic,
        (ControlKind::Static, WM_CTLCOLOREDIT) => {
            warn!("[Paint] ControlKind::Static received WM_CTLCOLOREDIT; using default route");
            PaintRoute::Default
        }
        _ => PaintRoute::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_routes_static_and_edit_messages_to_edit() {
        assert_eq!(
            resolve_paint_route(ControlKind::Edit, WM_CTLCOLORSTATIC),
            PaintRoute::Edit
        );
        assert_eq!(
            resolve_paint_route(ControlKind::Edit, WM_CTLCOLOREDIT),
            PaintRoute::Edit
        );
    }

    #[test]
    fn static_routes_static_message_to_label_and_edit_to_default() {
        assert_eq!(
            resolve_paint_route(ControlKind::Static, WM_CTLCOLORSTATIC),
            PaintRoute::LabelStatic
        );
        assert_eq!(
            resolve_paint_route(ControlKind::Static, WM_CTLCOLOREDIT),
            PaintRoute::Default
        );
    }

    #[test]
    fn combobox_routes_listbox_message_to_combo_listbox() {
        assert_eq!(
            resolve_paint_route(ControlKind::ComboBox, WM_CTLCOLORLISTBOX),
            PaintRoute::ComboListBox
        );
    }
}
