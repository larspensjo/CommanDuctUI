use windows::Win32::Graphics::Gdi::{HDC, HGDIOBJ, SelectObject};

/// RAII guard that selects a GDI object into an HDC and restores the previous
/// selection on [`Drop`].
///
/// This prevents the class of bugs where a manual `SelectObject` restore is
/// omitted on an early return or across a code refactor.
///
/// # Example
///
/// ```rust,ignore
/// let _font = unsafe { SelectedObject::select(hdc, my_font) };
/// // hdc now has my_font selected for the rest of this scope.
/// // The previous font is restored automatically when _font drops.
/// ```
pub(crate) struct SelectedObject {
    hdc: HDC,
    prev: HGDIOBJ,
}

impl SelectedObject {
    /// Selects `obj` into `hdc`, returning a guard that restores the previous
    /// selection on drop.
    ///
    /// # Safety
    ///
    /// `hdc` must be a valid device context and `obj` must be a valid GDI
    /// object compatible with `hdc`. Both must remain valid for the lifetime of
    /// the returned guard.
    pub(crate) unsafe fn select(hdc: HDC, obj: HGDIOBJ) -> Self {
        let prev = unsafe { SelectObject(hdc, obj) };
        Self { hdc, prev }
    }
}

impl Drop for SelectedObject {
    fn drop(&mut self) {
        unsafe { SelectObject(self.hdc, self.prev) };
    }
}
