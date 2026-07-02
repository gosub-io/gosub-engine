use crate::render::backend::{SurfaceRect, SurfaceSize};

/// Process-wide device-pixel ratio, set once by the host display thread (GTK/winit)
/// before rendering begins. Backends that rasterize at physical pixels (Cairo) read it;
/// backends that rasterize at CSS pixels (Skia, Vello) treat it as 1.
pub static DEVICE_PIXEL_RATIO: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

#[derive(Debug, Copy, Clone)]
pub struct DevicePixelRatio(pub f64);

/// Viewport definition for rendering.
///
/// A [`Viewport`] describes the rectangular region of a page that should be
/// rendered, defined by its top-left corner `(x, y)` and pixel `width`/`height`.
#[derive(Clone, Eq, PartialEq, Copy, Default)]
pub struct Viewport {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for Viewport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Viewport {{ x: {}, y: {}, width: {}, height: {} }}",
            self.x, self.y, self.width, self.height
        )
    }
}

impl Viewport {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    pub fn translate(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
    }

    pub fn aspect_ratio(&self) -> f32 {
        if self.height == 0 {
            0.0
        } else {
            self.width as f32 / self.height as f32
        }
    }

    pub fn as_size(&self) -> SurfaceSize {
        SurfaceSize {
            width: self.width,
            height: self.height,
        }
    }

    pub fn to_surface_rect(self, dpr: DevicePixelRatio) -> SurfaceRect {
        let fx = ((self.x as f64) * dpr.0).round();
        let fy = ((self.y as f64) * dpr.0).round();
        let fw = ((self.width as f64) * dpr.0).round().clamp(1.0, u32::MAX as f64);
        let fh = ((self.height as f64) * dpr.0).round().clamp(1.0, u32::MAX as f64);

        SurfaceRect {
            x: fx as i32,
            y: fy as i32,
            width: fw as u32,
            height: fh as u32,
        }
    }

    pub fn to_surface_size(self, dpr: DevicePixelRatio) -> SurfaceSize {
        let w = ((self.width as f64) * dpr.0).round().clamp(1.0, u32::MAX as f64) as u32;
        let h = ((self.height as f64) * dpr.0).round().clamp(1.0, u32::MAX as f64) as u32;
        SurfaceSize { width: w, height: h }
    }
}
