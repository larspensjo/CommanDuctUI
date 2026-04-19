//! Small helpers for Win32-facing safety boundaries.
//!
//! `catch_unwind_ffi` prevents Rust panics from unwinding through FFI entry
//! points, and `DeferredWindowState` supports one-time state transfer during
//! `WM_NCCREATE`.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

use windows::Win32::{Foundation::LPARAM, UI::WindowsAndMessaging::CREATESTRUCTW};

/// Runs an FFI callback body behind `catch_unwind` and falls back to a default
/// return value after logging if the body panics.
pub(crate) fn catch_unwind_ffi<T, F, D>(entry_point: &str, body: F, default: D) -> T
where
    F: FnOnce() -> T,
    D: FnOnce() -> T,
{
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(payload) => {
            log::error!(
                "FFI entry point '{entry_point}' panicked: {}",
                panic_payload_summary(payload.as_ref())
            );
            default()
        }
    }
}

fn panic_payload_summary(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Temporary owner for control state passed through `CREATESTRUCTW::lpCreateParams`.
#[derive(Debug)]
pub(crate) struct DeferredWindowState<T> {
    state: Option<Box<T>>,
}

impl<T> DeferredWindowState<T> {
    /// Boxes state so it can be handed across the `CreateWindowExW` boundary.
    pub(crate) fn new(state: T) -> Self {
        Self {
            state: Some(Box::new(state)),
        }
    }

    /// Converts the deferred owner into a raw pointer for `lpCreateParams`.
    pub(crate) fn into_raw(self) -> *mut Self {
        Box::into_raw(Box::new(self))
    }

    /// Takes ownership of the inner state from a `WM_NCCREATE` lparam payload.
    pub(crate) unsafe fn adopt_from_create_lparam(lparam: LPARAM) -> Option<*mut T> {
        let create_struct = unsafe { (lparam.0 as *const CREATESTRUCTW).as_ref()? };
        let create_params = create_struct.lpCreateParams as *mut Self;
        if create_params.is_null() {
            return None;
        }

        unsafe { &mut *create_params }
            .state
            .take()
            .map(Box::into_raw)
    }

    /// Drops the deferred owner if window creation fails before adoption.
    pub(crate) unsafe fn reclaim(raw: *mut Self) {
        if !raw.is_null() {
            let _ = unsafe { Box::from_raw(raw) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catch_unwind_ffi_returns_default_after_panic() {
        let result = catch_unwind_ffi("test_entry", || -> i32 { panic!("boom") }, || 7);

        assert_eq!(result, 7);
    }

    #[test]
    fn deferred_window_state_reclaim_drops_unadopted_state() {
        #[derive(Debug)]
        struct DropFlag<'a>(&'a std::cell::Cell<bool>);

        impl Drop for DropFlag<'_> {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        let dropped = std::cell::Cell::new(false);
        let raw = DeferredWindowState::new(DropFlag(&dropped)).into_raw();

        unsafe { DeferredWindowState::reclaim(raw) };

        assert!(dropped.get());
    }
}
