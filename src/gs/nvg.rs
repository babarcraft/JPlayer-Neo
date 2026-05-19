use nanovg_sys::{nvgBeginFrame, nvgBeginPath, nvgCreateGL3, nvgEndFrame, nvgFill, nvgFillPaint, nvgImagePattern, nvgRect, nvglCreateImageFromHandleGL3, NVGpaint};
use crate::gs::texture::Texture;

pub struct NvgContext {
    context: *mut nanovg_sys::NVGcontext
}

impl NvgContext {
    pub fn new() -> NvgContext {
        unsafe {
            let context = nvgCreateGL3(0);
            NvgContext { context }
        }
    }

    pub fn frame<F>(&mut self, size: (f32, f32), draw: F) where F: FnOnce(&mut NvgContext) {
        unsafe {
            nvgBeginFrame(self.context, size.0, size.1, 1.0);
            draw(self);
            nvgEndFrame(self.context);
        }
    }
    
    pub fn begin_path(&mut self) {
        unsafe {
            nvgBeginPath(self.context);
        }
    }
    
    pub fn create_texture_image(&mut self, texture: &Texture) -> i32 {
        unsafe {
            nvglCreateImageFromHandleGL3(self.context, texture.id, texture.width.unwrap() as i32, texture.height.unwrap() as i32, 0)
        }
    }
    
    pub fn image_paint(&mut self, image_id: i32, origin: (f32, f32), size: (f32, f32)) -> NVGpaint {
        unsafe {
            nvgImagePattern(self.context, origin.0, origin.1, size.0, size.1, 0.0, image_id, 1.0)
        }
    }
    
    pub fn set_fill_paint(&mut self, paint: NVGpaint) {
        unsafe {
            nvgFillPaint(self.context, paint);
        }
    }
    
    pub fn fill(&mut self) {
        unsafe {
            nvgFill(self.context);
        }
    }
    
    pub fn rect(&mut self, origin: (f32, f32), size: (f32, f32)) {
        unsafe {
            nvgRect(self.context, origin.0, origin.1, size.0, size.1);
        }
    }
}