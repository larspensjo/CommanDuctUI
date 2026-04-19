//! Named saturating conversions for Win32 boundary values.

/// Saturating `usize -> i32` conversion for Win32 count and index fields.
pub(crate) fn i32_from_usize_saturating(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Saturating `usize -> u32` conversion for Win32 size and page fields.
pub(crate) fn u32_from_usize_saturating(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{i32_from_usize_saturating, u32_from_usize_saturating};

    #[test]
    fn i32_conversion_saturates_large_values() {
        assert_eq!(i32_from_usize_saturating(usize::MAX), i32::MAX);
    }

    #[test]
    fn u32_conversion_saturates_large_values() {
        assert_eq!(u32_from_usize_saturating(usize::MAX), u32::MAX);
    }
}
