use crate::window_common::ControlKind;
use log::{debug, warn};
use windows::Win32::UI::WindowsAndMessaging::{
    WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaintRoute {
    LabelStatic,
    Edit,
    ComboListBox,
    Button,
    Default,
}

pub(crate) fn resolve_paint_route(kind: ControlKind, msg: u32) -> PaintRoute {
    match (kind, msg) {
        (ControlKind::Edit, WM_CTLCOLORSTATIC) => {
            debug!("[Paint] ControlKind::Edit routed via WM_CTLCOLORSTATIC to edit styling");
            PaintRoute::Edit
        }
        (ControlKind::Edit, WM_CTLCOLOREDIT) => PaintRoute::Edit,
        (ControlKind::ComboBox, WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT) => {
            debug!("[Paint] ControlKind::ComboBox routed {msg:#x} to combo listbox styling");
            PaintRoute::ComboListBox
        }
        (ControlKind::Button, WM_CTLCOLORBTN) => {
            debug!("[Paint] ControlKind::Button routed WM_CTLCOLORBTN to button styling");
            PaintRoute::Button
        }
        (ControlKind::RadioButton, WM_CTLCOLORBTN) => {
            debug!("[Paint] ControlKind::RadioButton routed WM_CTLCOLORBTN to button styling");
            PaintRoute::Button
        }
        (ControlKind::RadioButton, WM_CTLCOLORSTATIC) => {
            debug!("[Paint] ControlKind::RadioButton routed WM_CTLCOLORSTATIC to button styling");
            PaintRoute::Button
        }
        (ControlKind::CheckBox, WM_CTLCOLORBTN) => {
            debug!("[Paint] ControlKind::CheckBox routed WM_CTLCOLORBTN to button styling");
            PaintRoute::Button
        }
        (ControlKind::CheckBox, WM_CTLCOLORSTATIC) => {
            debug!("[Paint] ControlKind::CheckBox routed WM_CTLCOLORSTATIC to button styling");
            PaintRoute::Button
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

    #[test]
    fn combobox_routes_static_message_to_combo_listbox() {
        assert_eq!(
            resolve_paint_route(ControlKind::ComboBox, WM_CTLCOLORSTATIC),
            PaintRoute::ComboListBox,
            "CBS_DROPDOWNLIST sends WM_CTLCOLORSTATIC for the closed combo face"
        );
    }

    #[test]
    fn combobox_routes_edit_message_to_combo_listbox() {
        assert_eq!(
            resolve_paint_route(ControlKind::ComboBox, WM_CTLCOLOREDIT),
            PaintRoute::ComboListBox,
            "Some systems/themes route the closed combo face through WM_CTLCOLOREDIT"
        );
    }

    #[test]
    fn button_routes_btn_message_to_button() {
        assert_eq!(
            resolve_paint_route(ControlKind::Button, WM_CTLCOLORBTN),
            PaintRoute::Button
        );
    }

    #[test]
    fn radiobutton_routes_btn_message_to_button() {
        assert_eq!(
            resolve_paint_route(ControlKind::RadioButton, WM_CTLCOLORBTN),
            PaintRoute::Button
        );
    }

    #[test]
    fn radiobutton_routes_static_message_to_button() {
        assert_eq!(
            resolve_paint_route(ControlKind::RadioButton, WM_CTLCOLORSTATIC),
            PaintRoute::Button,
            "some radio-button paint paths surface as WM_CTLCOLORSTATIC"
        );
    }

    #[test]
    fn checkbox_routes_btn_message_to_button() {
        assert_eq!(
            resolve_paint_route(ControlKind::CheckBox, WM_CTLCOLORBTN),
            PaintRoute::Button
        );
    }

    #[test]
    fn checkbox_routes_static_message_to_button() {
        assert_eq!(
            resolve_paint_route(ControlKind::CheckBox, WM_CTLCOLORSTATIC),
            PaintRoute::Button,
            "checkbox paint paths can surface as WM_CTLCOLORSTATIC"
        );
    }
}
