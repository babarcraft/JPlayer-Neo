use std::ffi;
use std::ffi::{c_int, CStr, CString};
use std::fs::File;
use std::io::Read;
use std::ops::Range;
use std::os::raw::c_char;
use std::process::Command;
use std::ptr::{null, null_mut};
use std::str::FromStr;
use mlua::UserData;
use nanovg_sys::{nvgBeginFrame, nvgBeginPath, nvgCircle, nvgCreateFont, nvgCreateGL3, nvgCreateImage, nvgCreateImageRGBA, nvgDeleteImage, nvgEndFrame, nvgFill, nvgFillColor, nvgFillPaint, nvgFontFace, nvgFontSize, nvgImagePattern, nvgImageSize, nvgIntersectScissor, nvgLineTo, nvgMoveTo, nvgRect, nvgRestore, nvgRotate, nvgSave, nvgScale, nvgScissor, nvgStroke, nvgStrokeColor, nvgStrokeWidth, nvgText, nvgTextGlyphPositions, nvgTextMetrics, nvgTranslate, nvglCreateImageFromHandleGL3, NVGcolor, NVGglyphPosition, NVGpaint};
use crate::gs::texture::Texture;

pub struct NvgContext {
    context: *mut nanovg_sys::NVGcontext,
    size: (f32, f32),
    text_matrics: TextMatrics
}

#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub x: f32,
    pub y: f32
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }
}

impl Into<(f32, f32)> for Point {
    fn into(self) -> (f32, f32) {
        (self.x, self.y)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Color(NVGcolor);

impl Color {

    pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color(NVGcolor {
            rgba: [r, g, b, a],
        })
    }

    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::rgba(r, g, b, 1.0)
    }

    pub fn alpha(&self, a: f32) -> Self {
        let mut color = self.0;
        color.rgba[3] = a;
        Self(color)
    }

    pub fn gray(g: f32, a: f32) -> Self {
        Self::rgba(g, g, g, a)
    }

}

#[derive(Debug, Copy, Clone)]
pub enum Shape {
    Rect(f32, f32, f32, f32),
    Circle(f32, f32, f32),
}

impl Shape {

    pub fn intersects(&self, point: Point) -> bool {
        let (x0, y0, x1, y1) = self.bounds_absolute();
        let (x, y) = point.into();
        x >= x0 && y >= y0 && x <= x1 && y <= y1
    }

    pub fn bounds_absolute(&self) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.bounds();
        (x, y, x + w, y + h)
    }

    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        match *self {
            Shape::Rect(x0, y0, w, h) => {
                (x0, y0, w, h)
            }
            Shape::Circle(x0, y0, r) => {
                (x0 - r, y0 - r, r * 2.0, y0 * 2.0)
            }
        }
    }

    pub fn translate(&self, ox: f32, oy: f32) -> Shape {
        match *self {
            Shape::Rect(x, y, w, h) => {
                Shape::Rect(x + ox, y + oy, w, h)
            }
            Shape::Circle(x0, y0, r) => {
                Shape::Circle(x0 + ox, y0 + oy, r * 2.0)
            }
        }
    }
    
    pub fn scale_xy(&self, sx: f32, sy: f32, centered: bool) -> Shape {
        match *self {
            Shape::Rect(x, y, w, h) => {
                if centered {
                    let ox = (1.0 - sx) * w * 0.5;
                    let oy = (1.0 - sy) * h * 0.5;
                    Shape::Rect(x + ox, y + oy, w * sx, h * sy)
                } else {
                    Shape::Rect(x, y, w * sx, h * sy)
                }
            }
            _ => unimplemented!(),
        }
    }

    pub fn scale(&self, s: f32, centered: bool) -> Shape {
        match *self {
            Shape::Rect(x, y, w, h) => {
                if centered {
                    let ox = (1.0 - s) * w * 0.5;
                    let oy = (1.0 - s) * h * 0.5;
                    Shape::Rect(x + ox, y + oy, w * s, h * s)
                } else {
                    Shape::Rect(x, y, w * s, h * s)
                }
            }
            Shape::Circle(x0, y0, r) => {
                Shape::Circle(x0, y0, r * s)
            }
        }
    }

    pub fn with_padding(&self, padding: f32, centered: bool) -> Shape {
        match *self {
            Shape::Rect(x, y, w, h) => {
                if centered {
                    Shape::Rect(x + padding / 2.0, y + padding / 2.0, w - padding, h - padding)
                } else {
                    Shape::Rect(x, y, w + padding, h + padding)
                }
            }
            Shape::Circle(x0, y0, r) => {
                Shape::Circle(x0, y0, r + padding)
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum TextHorizontalAlignment {
    Left,
    Center,
    Right,
}

#[derive(Copy, Clone, Debug)]
pub enum TextVerticalAlignment {
    Top,
    Center,
    Bottom,
}

#[derive(Clone, Debug)]
pub struct Text {
    font: String,
    font_size: f32,

    data: String,
    glyph_positions: Vec<NVGglyphPosition>,
    matrics: TextMatrics,
    dirty: bool,
    fit: bool,
    pub(crate) x: f32,
    pub(crate) y: f32,
}

#[derive(Copy, Clone, Debug)]
pub struct TextMatrics {
    ascender: f32,
    descender: f32,
    line_height: f32,
}

impl TextMatrics {
    fn empty() -> TextMatrics {
        TextMatrics { ascender: 0.0, descender: 0.0, line_height: 0.0 }
    }
}

impl Text {
    
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let w = self.glyph_positions.last()
            .zip(self.glyph_positions.first())
            .map(|(last, first)| {
                let min = last.minx.min(first.minx);
                let max = last.maxx.max(first.maxx);
                max - min
            })
            .unwrap_or(0.0);
        (self.x, self.y - self.matrics.descender, w, self.matrics.line_height)
    }

    pub fn bounds_absolute(&self) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.bounds();
        (x, y, x + w, y + h)
    }

    pub fn char_bounds(&self, index: usize) -> Option<(f32, f32, f32, f32)> {
        if index == self.glyph_positions.len() {
            if index == 0 && self.glyph_positions.is_empty() {
                return Some((self.x, self.y - self.matrics.descender, 0.0, self.matrics.line_height));
            }

            let glyph = self.glyph_positions.get(index - 1)?;
            let maxx = glyph.maxx;
            return Some((self.x + maxx, self.y - self.matrics.descender, glyph.maxx - maxx, self.matrics.line_height))
        }
        let glyph = self.glyph_positions.get(index)?;
        let minx = glyph.minx;
        Some((self.x + minx, self.y - self.matrics.descender, glyph.maxx - minx, self.matrics.line_height))
    }

    pub fn char_range_bounds(&self, range: Range<usize>) -> Option<(f32, f32, f32, f32)> {
        let (x0, y0, xf0, yf0) = self.char_bounds_absolute(range.start)?;
        let (x, y, xf, yf) = self.char_bounds_absolute(range.end)?;
        let w = xf - x0;
        let h = yf - y0;
        Some((x0, y0, w, h))
    }

    pub fn char_range_bounds_absolute(&self, range: Range<usize>) -> Option<(f32, f32, f32, f32)> {
        let (x, y, w, h) = self.char_range_bounds(range)?;
        Some((x, y, x + w, y + h))
    }

    pub fn char_bounds_absolute(&self, index: usize) -> Option<(f32, f32, f32, f32)> {
        let (x, y, w, h) = self.char_bounds(index)?;
        Some((x, y, x + w, y + h))
    }

    pub fn translate(&mut self, ox: f32, oy: f32) {
        self.x += ox;
        self.y += oy;
    }

    pub fn set_position(&mut self, x: f32, y: f32) {
        self.x = x;
        self.y = y;
    }
    
    pub fn push_back(&mut self, c: char) {
        self.data.push(c);
        self.dirty = true;
    }

    pub fn insert_at(&mut self, index: usize, c: char) {
        self.data.insert(index, c);
        self.dirty = true;
    }

    pub fn remove_at(&mut self, index: usize) {
        self.data.remove(index);
        self.glyph_positions.remove(index);
        self.dirty = true;
    }

    pub fn push_str(&mut self, str: &str) {
        self.data.extend(str.chars());
        self.dirty = true;
    }

    pub fn clear(&mut self) {
        self.data.clear();
        self.glyph_positions.clear();
        self.dirty = true;
    }
    
    pub fn pop(&mut self) -> Option<(char, NVGglyphPosition)> {
        let char = self.data.pop().zip(self.glyph_positions.pop());
        self.dirty = true;
        char
    }

    pub fn split_at(&mut self, index: usize) -> Option<Self> {
        let offset = self.glyph_positions.get(index).map(|glyph| glyph.minx).unwrap_or(0.0);
        let num = self.data.as_bytes().len() - index;
        let mut new_glyphs = Vec::with_capacity(num);
        for _ in 0..num {
            let mut glyph = self.glyph_positions.pop()?;
            glyph.x -= offset;
            glyph.minx -= offset;
            glyph.maxx -= offset;
            self.glyph_positions.push(glyph);
        }
        new_glyphs.reverse();
        let data = self.data.split_off(index);
        Some(Self {
            font: self.font.clone(),
            font_size: self.font_size,
            data,
            glyph_positions: new_glyphs,
            matrics: self.matrics,
            dirty: true,
            fit: false,
            x: self.x,
            y: self.y,
        })
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

}

#[derive(Debug)]
pub struct Image {
    id: c_int,
    pub width: u32,
    pub height: u32,
}

impl Image {

    pub fn size_conserve_aspect_ratio(&self, w: f32, h: f32) -> (f32, f32) {
        let sw = self.width as f32;
        let sh = self.height as f32;
        let s = (w / sw).min(h / sh);
        (sw * s, sh * s)
    }

}

impl UserData for Image {}

pub fn string_to_ptr_end(string: &str) -> (*const c_char, *const c_char) {
    unsafe {
        let ptr = string.as_ptr() as *const c_char;
        let end = ptr.add(string.len());
        (ptr, end)
    }
}

pub enum DrawCommand<'a> {
    TextBox(Shape, &'a Text, Color, TextHorizontalAlignment, TextVerticalAlignment),
    ShapeColor(Shape, Color),
    ShapeImage(Shape, &'a Image),
}

impl NvgContext {
    pub fn new() -> NvgContext {
        unsafe {
            let context = nvgCreateGL3(0);
            NvgContext { context, size: (0.0, 0.0), text_matrics: TextMatrics::empty() }
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
            nvgTextMetrics(
                self.context,
                &mut self.text_matrics.ascender,
                &mut self.text_matrics.descender,
                &mut self.text_matrics.line_height
            );
            self.text_matrics.descender = self.text_matrics.descender.abs();
            self.text_matrics.ascender = self.text_matrics.ascender.abs();
            self.text_matrics.line_height = self.text_matrics.line_height.abs();
        }
    }

    pub fn draw_text(&mut self, text: &Text) {
        unsafe {
            let data = &text.data;
            let (ptr, end) = string_to_ptr_end(data);
            self.set_font(text.font.as_str(), text.font_size);
            nvgText(self.context, text.x, self.invert_y(text.y, 0.0), ptr, end);
        }
    }

    pub fn draw_text_inside(&mut self, text: &Text, shape: Shape, horizontal: TextHorizontalAlignment, vertical: TextVerticalAlignment) {
        let (sx, sy, sw, sh) = shape.bounds();
        let (tx, ty, tw, th) = text.bounds();
        let dx = sx - tx;
        let dy = sy - ty;
        let ox = (sw - tw) * match horizontal {
            TextHorizontalAlignment::Left => 0.0,
            TextHorizontalAlignment::Center => 0.5,
            TextHorizontalAlignment::Right => 1.0,
        };
        let oy = (sh - th) * match vertical {
            TextVerticalAlignment::Top => 0.0,
            TextVerticalAlignment::Center => 0.5,
            TextVerticalAlignment::Bottom => 1.0,
        };
        unsafe {
            let (ptr, end) = string_to_ptr_end(&text.data);
            nvgFontSize(self.context, text.font_size);
            nvgText(self.context, dx + ox, self.invert_y(dy + oy, 0.0), ptr, end);
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
    
    pub fn update_text(&mut self, text: &mut Text) {
        self.set_font(text.font.as_str(), text.font_size);
        text.matrics = self.text_matrics;
        if text.glyph_positions.len() == text.data.len() && !text.dirty {
            return;
        }
        for _ in text.glyph_positions.len()..text.data.len() {
            text.glyph_positions.push(
                NVGglyphPosition {
                    s: null(),
                    x: 0.0,
                    minx: 0.0,
                    maxx: 0.0,
                }
            )
        }
        unsafe {
            let glyph_positions = &mut text.glyph_positions;
            let text = &text.data;
            let ptr = text.as_ptr() as *const c_char;
            let end = text.as_ptr().add(text.len()) as *const c_char;
            nvgTextGlyphPositions(
                self.context,
                0.0,
                0.0,
                ptr,
                end,
                glyph_positions[..].as_mut_ptr(),
                glyph_positions.len() as i32
            );
        }
        text.dirty = false
    }

    pub fn fit_text(&mut self, text: &mut Text, pw: f32, ph: f32) {
        if text.fit && !text.dirty {
            return
        }
        let mut bigger = false;
        loop {
            text.dirty = true;
            self.update_text(text);
            let (x, y, w, h) = text.bounds();
            if w > pw || h > ph {
                bigger = true;
                text.font_size -= 1.0;
            } else if bigger {
                break
            } else {
                bigger = false;
                text.font_size += 1.0;
            }
        }
        text.fit = true;
        text.dirty = false;
    }

    pub fn text(&mut self, text: &str, font: &str, font_size: f32) -> Text {
        self.set_font(font, font_size);
        let mut glyph_positions = (0..text.len()).map(|_| {
            NVGglyphPosition {
                s: null(),
                x: 0.0,
                minx: 0.0,
                maxx: 0.0,
            }
        }).collect::<Vec<NVGglyphPosition>>();
        unsafe {
            let ptr = text.as_ptr() as *const c_char;
            let end = text.as_ptr().add(text.len()) as *const c_char;
            nvgTextGlyphPositions(
                self.context,
                0.0,
                0.0,
                ptr,
                end,
                glyph_positions[..].as_mut_ptr(),
                glyph_positions.len() as i32
            );
            Text {
                font: font.to_string(),
                font_size,
                data: text.to_string(),
                glyph_positions,
                matrics: self.text_matrics.clone(),
                dirty: true,
                fit: false,
                x: 0.0,
                y: 0.0
            }
        }
    }

    pub fn text_matrics(&self) {
        unsafe {
            let mut matrics = TextMatrics {
                ascender: 0.0,
                descender: 0.0,
                line_height: 0.0,
            };
            nvgTextMetrics(self.context, &mut matrics.ascender, &mut matrics.descender, &mut matrics.line_height);
        }
    }
    
    pub fn create_texture_image(&mut self, texture: &Texture) -> Option<Image> {
        unsafe {
            let image = nvglCreateImageFromHandleGL3(
                self.context,
                texture.id,
                texture.width? as i32,
                texture.height? as i32,
                nanovg_sys::NVGimageFlagsGL::NVG_IMAGE_NODELETE.bits()
            );
            Some(Image { id: image, width: texture.width?, height: texture.height? })
        }
    }
    
    pub fn image_paint(&mut self, image: &Image, shape: Shape, alpha: f32) -> NVGpaint {
        unsafe {
            let (x, y, w, h) = shape.bounds();
            nvgImagePattern(self.context, x, self.invert_y(y, h), w, h, 0.0, image.id, alpha)
        }
    }

    pub fn fill_color(&mut self, color: Color) {
        unsafe {
            nvgFillColor(self.context, color.0);
        }
    }
    
    pub fn fill_paint(&mut self, paint: NVGpaint) {
        unsafe {
            nvgFillPaint(self.context, paint);
        }
    }
    
    pub fn fill(&mut self) {
        unsafe {
            nvgFill(self.context);
        }
    }

    pub fn stroke(&mut self) {
        unsafe {
            nvgStroke(self.context);
        }
    }

    pub fn rotate(&mut self, angle: f32) {
        unsafe {
            nvgRotate(self.context, angle);
        }
    }

    pub fn translate(&mut self, x: f32, y: f32) {
        unsafe {
            nvgTranslate(self.context, x, y);
        }
    }

    pub fn scale(&mut self, x: f32, y: f32) {
        unsafe {
            nvgScale(self.context, x, y);
        }
    }

    pub fn save_state(&mut self) {
        unsafe {
            nvgSave(self.context);
        }
    }

    pub fn restore_state(&mut self) {
        unsafe {
            nvgRestore(self.context);
        }
    }

    pub fn stroke_color(&mut self, color: Color) {
        unsafe {
            nvgStrokeColor(self.context, color.0);
        }
    }

    pub fn stroke_width(&mut self, width: f32) {
        unsafe {
            nvgStrokeWidth(self.context, width);
        }
    }

    fn invert_y(&self, y: f32, h: f32) -> f32 {
        self.size.1 - y - h
    }

    pub fn draw_shape(&mut self, shape: Shape) {
        unsafe {
            match shape {
                Shape::Rect(x, y, w, h) => {
                    nvgRect(self.context, x, self.invert_y(y, h), w, h);
                }
                Shape::Circle(x, y, r) => {
                    nvgCircle(self.context, x, self.invert_y(y, r * 2.0), r);
                }
            }
        }
    }

    pub fn fill_shape_color(&mut self, shape: Shape, color: Color) {
        self.begin_path();
        self.draw_shape(shape);
        self.fill_color(color);
        self.fill();
    }

    pub fn draw_line(&mut self, begin: Point, end: Point) {
        unsafe {
            nvgMoveTo(self.context, begin.x, self.invert_y(begin.y, 0.0));
            nvgLineTo(self.context, end.x, self.invert_y(end.y, 0.0));
        }
    }

    pub fn create_image(&mut self, path: &str) -> Option<Image> {
        unsafe {
            let path = CString::new(path).unwrap();
            let mut width = 0;
            let mut height = 0;
            let image = nvgCreateImage(self.context, path.as_ptr(), 0);
            nvgImageSize(self.context, image, &mut width, &mut height);
            Some(image).take_if(|id| *id >= 0).map(|id| Image {
                id, width: width as u32, height: height as u32
            })
        }
    }

    pub fn delete_image(&mut self, image: Image) {
        unsafe {
            nvgDeleteImage(self.context, image.id);
        }
    }

    pub fn create_image_webp(&mut self, data: &[u8]) -> Option<i32> {
        unsafe {
            let (width, height, buf) = libwebp::WebPDecodeRGBA(data).ok()?;
            let data_ptr = buf.as_ptr();
            Some(nvgCreateImageRGBA(self.context, width as i32, height as i32, 0, data_ptr))
                .take_if(|id| *id >= 0)
        }
    }
    
    pub fn width(&self, p: Option<f32>) -> f32 {
        self.size.0 * p.unwrap_or(1.0)
    }
    
    pub fn height(&self, p: Option<f32>) -> f32 {
        self.size.1 * p.unwrap_or(1.0)
    }
    
    pub fn relative(&self, px: f32, py: f32) -> (f32, f32) {
        (self.width(Some(px)), self.height(Some(py)))
    }
    
    pub fn handle_command(&mut self, command: DrawCommand) {
        match command {
            DrawCommand::ShapeColor(shape, color) => {
                self.begin_path();
                self.draw_shape(shape);
                self.fill_color(color);
                self.fill();
            }
            DrawCommand::ShapeImage(shape, image) => {
                self.begin_path();
                self.draw_shape(shape);
                let paint = self.image_paint(&image, shape, 1.0);
                self.fill_paint(paint);
                self.fill();
            }
            DrawCommand::TextBox(shape, text, color, horiz, vert) => {
                self.begin_path();
                self.fill_color(color);
                self.draw_text_inside(text, shape, horiz, vert);
                self.fill();
            }
        }
    }
}