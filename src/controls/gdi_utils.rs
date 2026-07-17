use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, HBITMAP, HDC,
    HGDIOBJ, SRCCOPY, SelectObject, SetViewportOrgEx,
};

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

/// Owns a compatible memory DC and bitmap for atomically painting a dirty client area.
///
/// The buffer DC has a viewport origin that maps normal client coordinates to its
/// small dirty-area bitmap. Dropping the guard restores the selected bitmap before
/// releasing the bitmap and DC, including on every early return.
pub(crate) struct PaintBuffer {
    target_hdc: HDC,
    memory_hdc: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
    dirty: RECT,
}

impl PaintBuffer {
    /// Creates a buffer exactly covering `dirty`, or returns `None` for an empty
    /// rectangle or an allocation failure.
    ///
    /// # Safety
    ///
    /// `target_hdc` must remain valid until this guard is dropped or presented.
    pub(crate) unsafe fn new(target_hdc: HDC, dirty: RECT) -> Option<Self> {
        let width = dirty.right - dirty.left;
        let height = dirty.bottom - dirty.top;
        if width <= 0 || height <= 0 {
            return None;
        }

        let memory_hdc = unsafe { CreateCompatibleDC(Some(target_hdc)) };
        if memory_hdc.is_invalid() {
            return None;
        }
        let bitmap = unsafe { CreateCompatibleBitmap(target_hdc, width, height) };
        if bitmap.is_invalid() {
            let _ = unsafe { DeleteDC(memory_hdc) };
            return None;
        }
        let previous_bitmap = unsafe { SelectObject(memory_hdc, bitmap.into()) };
        if previous_bitmap.is_invalid() {
            let _ = unsafe { DeleteObject(bitmap.into()) };
            let _ = unsafe { DeleteDC(memory_hdc) };
            return None;
        }
        if !unsafe { SetViewportOrgEx(memory_hdc, -dirty.left, -dirty.top, None) }.as_bool() {
            let _ = unsafe { SelectObject(memory_hdc, previous_bitmap) };
            let _ = unsafe { DeleteObject(bitmap.into()) };
            let _ = unsafe { DeleteDC(memory_hdc) };
            return None;
        }

        Some(Self {
            target_hdc,
            memory_hdc,
            bitmap,
            previous_bitmap,
            dirty,
        })
    }

    pub(crate) const fn hdc(&self) -> HDC {
        self.memory_hdc
    }

    /// Copies the fully rendered dirty buffer to the target in one `BitBlt`.
    ///
    /// # Safety
    ///
    /// The target DC supplied to [`Self::new`] must still be valid.
    pub(crate) unsafe fn present(&self) -> windows::core::Result<()> {
        let width = self.dirty.right - self.dirty.left;
        let height = self.dirty.bottom - self.dirty.top;
        if width <= 0 || height <= 0 {
            return Ok(());
        }
        unsafe {
            BitBlt(
                self.target_hdc,
                self.dirty.left,
                self.dirty.top,
                width,
                height,
                Some(self.memory_hdc),
                self.dirty.left,
                self.dirty.top,
                SRCCOPY,
            )
        }
    }
}

impl Drop for PaintBuffer {
    fn drop(&mut self) {
        unsafe {
            let _ = SelectObject(self.memory_hdc, self.previous_bitmap);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.memory_hdc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Gdi::{GetPixel, SetPixel};

    #[test]
    fn present_maps_non_origin_dirty_rect_to_target_coordinates() {
        const TARGET_WIDTH: i32 = 128;
        const TARGET_HEIGHT: i32 = 96;
        const MARKER: COLORREF = COLORREF(0x00FF_FFFF);

        unsafe {
            let target_hdc = CreateCompatibleDC(None);
            assert!(!target_hdc.is_invalid());
            let target_bitmap = CreateCompatibleBitmap(target_hdc, TARGET_WIDTH, TARGET_HEIGHT);
            assert!(!target_bitmap.is_invalid());

            let presented_pixel = {
                let _target_bitmap = SelectedObject::select(target_hdc, target_bitmap.into());
                let dirty = RECT {
                    left: 10,
                    top: 40,
                    right: 90,
                    bottom: 70,
                };
                let marker_x = dirty.left + 3;
                let marker_y = dirty.top + 4;
                let buffer = PaintBuffer::new(target_hdc, dirty).expect("paint buffer");

                assert_eq!(SetPixel(buffer.hdc(), marker_x, marker_y, MARKER), MARKER);
                buffer.present().expect("present paint buffer");
                GetPixel(target_hdc, marker_x, marker_y)
            };

            let _ = DeleteObject(target_bitmap.into());
            let _ = DeleteDC(target_hdc);
            assert_eq!(presented_pixel, MARKER);
        }
    }
}
