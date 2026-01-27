use crate::window_common::ControlKind;
use log::{debug, warn};
use windows::Win32::UI::WindowsAndMessaging::{WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaintRoute {
    LabelStatic,
    Edit,
    Default,
}

pub(crate) fn resolve_paint_route(kind: ControlKind, msg: u32) -> PaintRoute {
    match (kind, msg) {
        (ControlKind::Edit, WM_CTLCOLORSTATIC) => {
            debug!("[Paint] ControlKind::Edit routed via WM_CTLCOLORSTATIC to edit styling");
            PaintRoute::Edit
        }
        (ControlKind::Edit, WM_CTLCOLOREDIT) => PaintRoute::Edit,
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
}
