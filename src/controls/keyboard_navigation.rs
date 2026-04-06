use windows::Win32::Foundation::{HWND, LRESULT};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{WINDOW_STYLE, WS_TABSTOP};

const DLGC_WANTARROWS: isize = 0x0001;

/// Declares which keyboard-navigation behaviors a custom child control opts into.
///
/// The goal is to encode the Win32 contract in one place:
/// - keyboard-navigable controls must be tab-stoppable
/// - a mouse activation should move focus into the control
/// - WM_GETDLGCODE should request dialog-managed navigation keys
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyboardNavigation {
    pub focus_on_click: bool,
    pub want_arrows: bool,
}

impl KeyboardNavigation {
    pub(crate) const DIALOG_NAVIGATION: Self = Self {
        focus_on_click: true,
        want_arrows: true,
    };

    pub(crate) const fn needs_tab_stop(self) -> bool {
        self.focus_on_click || self.want_arrows
    }

    pub(crate) const fn dialog_code_bits(self) -> isize {
        let mut bits = 0;
        if self.want_arrows {
            bits |= DLGC_WANTARROWS;
        }
        bits
    }
}

pub(crate) fn apply_window_style(
    base: WINDOW_STYLE,
    navigation: KeyboardNavigation,
) -> WINDOW_STYLE {
    if navigation.needs_tab_stop() {
        base | WS_TABSTOP
    } else {
        base
    }
}

pub(crate) unsafe fn focus_on_click(hwnd: HWND, navigation: KeyboardNavigation) {
    if navigation.focus_on_click {
        let _ = unsafe { SetFocus(Some(hwnd)) };
    }
}

pub(crate) fn dialog_code(navigation: KeyboardNavigation) -> Option<LRESULT> {
    let bits = navigation.dialog_code_bits();
    if bits == 0 { None } else { Some(LRESULT(bits)) }
}

#[cfg(test)]
mod tests {
    use super::{KeyboardNavigation, apply_window_style, dialog_code};
    use windows::Win32::UI::WindowsAndMessaging::{WINDOW_STYLE, WS_CHILD, WS_TABSTOP, WS_VISIBLE};

    #[test]
    fn dialog_navigation_adds_tab_stop_to_window_style() {
        let base = WS_CHILD | WS_VISIBLE;
        let style = apply_window_style(base, KeyboardNavigation::DIALOG_NAVIGATION);
        assert_ne!(style.0 & WS_TABSTOP.0, 0);
    }

    #[test]
    fn no_navigation_leaves_window_style_unchanged() {
        let base = WINDOW_STYLE(0x1234);
        let style = apply_window_style(
            base,
            KeyboardNavigation {
                focus_on_click: false,
                want_arrows: false,
            },
        );
        assert_eq!(style, base);
    }

    #[test]
    fn dialog_navigation_requests_arrow_keys() {
        let code = dialog_code(KeyboardNavigation::DIALOG_NAVIGATION).expect("dialog code");
        assert_eq!(code.0, 0x0001);
    }

    #[test]
    fn non_navigable_controls_have_no_dialog_code_override() {
        assert_eq!(
            dialog_code(KeyboardNavigation {
                focus_on_click: false,
                want_arrows: false,
            }),
            None
        );
    }
}
