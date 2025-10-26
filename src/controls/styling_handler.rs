/*
 * Helper utilities for translating styling primitives into Win32-friendly values.
 * The platform state now owns the heavier parsing logic; this module keeps the
 * lightweight conversions needed across multiple handlers.
 *
 * TODO: Should we deprecate this module?
 */

use crate::styling::Color;
use windows::Win32::Foundation::COLORREF;

/*
 * Creates a Win32 COLORREF from the platform-agnostic `Color` struct.
 * Win32 expects colors in BGR format, so this function handles the conversion.
 */
pub(crate) fn color_to_colorref(color: &Color) -> COLORREF {
    COLORREF((color.r as u32) | ((color.g as u32) << 8) | ((color.b as u32) << 16))
}
