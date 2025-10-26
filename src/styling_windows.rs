/*
 * Windows-specific styling implementation. This module augments the
 * platform-agnostic primitives with Win32 resource management and re-exports
 * those primitives so the rest of the codebase can use the same names
 * regardless of target platform.
 */

pub use super::styling_primitives::{Color, ControlStyle, FontDescription, FontWeight, StyleId};

use windows::Win32::Graphics::Gdi::{DeleteObject, HBRUSH, HFONT, HGDIOBJ};

/*
 * The `platform_layer`'s internal, processed representation of a `ControlStyle`.
 * This struct holds native resource handles (like `HFONT` and `HBRUSH`) that are
 * created from the platform-agnostic descriptions. This encapsulates Win32-specific
 * types and handles their cleanup via the `Drop` trait.
 */
#[derive(Debug, Clone)]
pub(crate) struct ParsedControlStyle {
    pub(crate) font_handle: Option<HFONT>,
    pub(crate) text_color: Option<Color>,
    pub(crate) background_color: Option<Color>,
    pub(crate) background_brush: Option<HBRUSH>,
    // ... other parsed properties ...
}

// SAFETY: Parsed styles are created, consumed, and destroyed on the platform thread.
// The contained Win32 handles (`HFONT`, `HBRUSH`) are plain value types that are only
// used under that thread's message loop, so sharing ownership via `Arc` is safe.
unsafe impl Send for ParsedControlStyle {}
unsafe impl Sync for ParsedControlStyle {}

impl Drop for ParsedControlStyle {
    /*
     * Ensures that native GDI resources, such as HFONTs and HBRUSHes, are properly
     * released when a `ParsedControlStyle` is no longer in use. This prevents
     * resource leaks.
     */
    fn drop(&mut self) {
        if let Some(hfont) = self.font_handle.take() {
            if !hfont.is_invalid() {
                // It's safe to call DeleteObject on a font handle.
                unsafe {
                    _ = DeleteObject(HGDIOBJ(hfont.0));
                }
            }
        }
        if let Some(hbrush) = self.background_brush.take() {
            if !hbrush.is_invalid() {
                // It's also safe to call DeleteObject on a brush handle.
                unsafe {
                    _ = DeleteObject(HGDIOBJ(hbrush.0));
                }
            }
        }
    }
}
