use std::ffi::{CStr, CString};
use std::ptr::{null, null_mut};
use std::str::FromStr;
use nanovg_sys::{nvgBeginFrame, nvgBeginPath, nvgCreateFont, nvgCreateGL3, nvgEndFrame, nvgFill, nvgFillColor, nvgFillPaint, nvgFontFace, nvgFontSize, nvgImagePattern, nvgRect, nvgText, nvglCreateImageFromHandleGL3, NVGcolor, NVGpaint};
use crate::gs::texture::Texture;

pub struct NvgContext {
    context: *mut nanovg_sys::NVGcontext,
    size: (f32, f32),
}

impl NvgContext {
    pub fn new() -> NvgContext {
        unsafe {
            let context = nvgCreateGL3(0);
            NvgContext { context, size: (0.0, 0.0) }
        }
    }

    pub fn load_font(&mut self, name: &str, path: &str) {
        unsafe {
            let name = CString::from_str(name).unwrap();
            let path = CString::from_str(path).unwrap();
            nvgCreateFont(self.context, name.as_ptr(), path.as_ptr());
        }
    }

    pub fn set_font(&mut self, name: &str, size: f32) {
        unsafe {
            let name = CString::from_str(name).unwrap();
            nvgFontFace(self.context, name.as_ptr());
            nvgFontSize(self.context, size);
        }
    }

    pub fn text(&mut self, origin: (f32, f32), text: &str) {
        unsafe {
            let text = CString::from_str(text).unwrap();
            nvgText(self.context, origin.0, origin.1, text.as_ptr(), null());
        }
    }

    pub fn frame<F>(&mut self, size: (f32, f32), draw: F) where F: FnOnce(&mut NvgContext) {
        unsafe {
            nvgBeginFrame(self.context, size.0, size.1, 1.0);
            self.size = size;
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
            let oy = self.size.1 - origin.1 - size.1;
            nvgImagePattern(self.context, origin.0, oy, size.0, size.1, 0.0, image_id, 1.0)
        }
    }

    pub fn fill_color(&mut self, color: (f32, f32, f32, f32)) {
        unsafe {
            nvgFillColor(self.context, NVGcolor {
                rgba: [color.0, color.1, color.2, color.3],
            });
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
            let y = self.size.1 - origin.1 - size.1;
            nvgRect(self.context, origin.0, y, size.0, size.1);
        }
    }
}