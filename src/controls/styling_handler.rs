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

/*
 * Converts a Win32 COLORREF (BGR) back to platform-agnostic Color (RGB).
 * Used for retrieving system colors and converting them to our Color type.
 */
pub(crate) fn colorref_to_color(cr: COLORREF) -> Color {
    Color {
        r: (cr.0 & 0xFF) as u8,
        g: ((cr.0 >> 8) & 0xFF) as u8,
        b: ((cr.0 >> 16) & 0xFF) as u8,
    }
}
