/*
 * Non-Windows styling stub. This module simply re-exports the platform-
 * agnostic styling primitives so that the rest of the codebase can compile
 * without pulling in any Win32-specific dependencies.
 */

pub use super::styling_primitives::{Color, ControlStyle, FontDescription, FontWeight, StyleId};
