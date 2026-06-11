use std::cell::RefCell;
use std::ffi;
use std::ffi::{c_int, c_void, CStr, CString};
use std::fs::File;
use std::io::Read;
use std::mem::MaybeUninit;
use std::ops::Range;
use std::os::raw::c_char;
use std::process::Command;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::str::FromStr;
use gl::types::GLsizei;
use libc::aio_return;
use mlua::UserData;
use nanovg_sys::{nvgBeginFrame, nvgBeginPath, nvgCircle, nvgCreateFont, nvgCreateGL3, nvgCreateImage, nvgCreateImageRGBA, nvgDeleteGL3, nvgDeleteImage, nvgEndFrame, nvgFill, nvgFillColor, nvgFillPaint, nvgFontFace, nvgFontSize, nvgImagePattern, nvgImageSize, nvgIntersectScissor, nvgLineTo, nvgMoveTo, nvgRect, nvgRestore, nvgRotate, nvgSave, nvgScale, nvgScissor, nvgStroke, nvgStrokeColor, nvgStrokeWidth, nvgText, nvgTextGlyphPositions, nvgTextMetrics, nvgTranslate, nvglCreateImageFromHandleGL3, NVGcolor, NVGcontext, NVGglyphPosition, NVGpaint};
use crate::gs::texture::Texture;

struct NvgContextInside(*mut NVGcontext);

impl NvgContextInside {
    fn new() -> NvgContextInside {
        unsafe {
            NvgContextInside(nvgCreateGL3(0))
        }
    }
}

impl Drop for NvgContextInside {
    fn drop(&mut self) {
        unsafe {
            nvgDeleteGL3(self.0);
        }
    }
}

#[derive(Clone)]
pub struct NvgInstance {
    cell: Rc<RefCell<NvgContextInside>>,
    context: *mut NVGcontext,
    size: (f32, f32),
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

    pub fn intersects(&self, point: (f32, f32)) -> bool {
        let (x0, y0, x1, y1) = self.bounds_absolute();
        let (x, y) = point;
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

#[derive(Copy, Clone)]
pub enum TextFitType {
    RangeAndScale,
    Range,
    Scale
}

#[derive(Copy, Clone)]
pub enum TextAlign {
    Right, Left,
    CenterVertical, CenterHorizontal,
    Top, Bottom
}

pub struct Text {
    instance: NvgInstance,
    text: String,
    font: (String, f32),
    glyphs: Vec<MaybeUninit<NVGglyphPosition>>,
    offset: usize,
    range: Option<Range<usize>>,
    pos: (f32, f32),
    fit_size: Option<(f32, f32)>,
    fit_type: TextFitType,
    metrics: TextMatrics,
    align: (TextAlign, TextAlign),
}

impl Text {

    pub fn new(instance: &NvgInstance, font: (&str, f32)) -> Self {
        let (font, font_size) = font;
        let metrics = instance.text_metrics();
        Self {
            instance: instance.clone(),
            text: String::new(),
            font: (font.to_string(), font_size),
            metrics,
            glyphs: vec![],
            range: None,
            offset: 0,
            pos: (0.0, 0.0),
            fit_size: None,
            fit_type: TextFitType::RangeAndScale,
            align: (TextAlign::CenterHorizontal, TextAlign::CenterVertical),
        }
    }

    pub fn set_pos(&mut self, pos: (f32, f32)) {
        self.pos = pos;
        self.update_glyphs();
        self.update();
    }

    pub fn get_pos(&self) -> (f32, f32) {
        self.pos
    }

    pub fn with_string<F>(&mut self, f: F)
        where F: Fn(&mut String) -> () {
        f(&mut self.text);
        self.update_glyphs();
        self.update();
    }

    pub fn set_text(&mut self, text: &str) {
        self.text.clear();
        self.text.push_str(text);
        self.update_glyphs();
        self.update();
    }

    pub fn get_text(&self) -> &str {
        self.text.as_str()
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn set_offset(&mut self, offset: usize) {
        self.offset = offset.max(0).min(self.len());
        self.update();
    }

    pub fn get_offset(&self) -> usize {
        self.offset
    }

    pub fn set_fit_type(&mut self, fit_type: TextFitType) {
        self.fit_type = fit_type;
        self.update();
    }

    pub fn set_font(&mut self, target_font: (&str, f32)) {
        let font = &mut self.font.0;
        font.clear();
        font.push_str(target_font.0);
        self.font.1 = target_font.1;
        self.update_glyphs();
        self.update();
    }

    pub fn get_fit_type(&self) -> TextFitType {
        self.fit_type
    }

    pub fn set_align(&mut self, align: (TextAlign, TextAlign)) {
        self.align = align;
    }

    pub fn get_align(&self) -> (TextAlign, TextAlign) {
        self.align
    }

    pub fn set_fit_size(&mut self, size: Option<(f32, f32)>) {
        self.fit_size = size;
        self.update();
    }

    pub fn get_fit_size(&self) -> Option<(f32, f32)> {
        self.fit_size
    }

    fn get_glyph(&self, index: usize) -> (f32, f32) {
        let slice = self.glyphs_slice();
        if slice.is_empty() {
            return (self.pos.0, self.pos.0)
        }
        let gl = if index >= slice.len() {
            slice.last().map(|gl| {
                (gl.maxx, gl.maxx)
            })
        } else {
            slice.get(index)
                .map(|gl| (gl.minx, gl.maxx))
        };
        let x = self.pos.0;
        gl.unwrap_or((x, x))
    }

    fn glyphs_slice(&self) -> &[NVGglyphPosition] {
        unsafe {
            std::slice::from_raw_parts(self.glyphs.as_ptr() as *mut NVGglyphPosition, self.len())
        }
    }

    fn glyphs_slice_mut(&mut self) -> &mut [NVGglyphPosition] {
        unsafe {
            std::slice::from_raw_parts_mut(self.glyphs.as_mut_ptr() as *mut NVGglyphPosition, self.len())
        }
    }

    pub fn update_glyphs(&mut self) {
        if self.text.len() > self.glyphs.len() {
            self.glyphs = vec![MaybeUninit::uninit(); self.text.len()];
        }
        unsafe {
            let ptr = self.glyphs.as_mut_ptr() as *mut NVGglyphPosition;
            let slice = std::slice::from_raw_parts_mut(ptr, self.len());
            self.instance_set_font();
            self.instance.text_glyph_positions(self.pos, &self.text, slice);
        }
    }

    fn full_width(&self) -> Option<f32> {
        let glyphs = self.glyphs_slice();
        glyphs.first()
            .zip(glyphs.last())
            .map(|(first, last)| last.maxx - first.minx)
    }

    fn instance_set_font(&mut self) {
        self.instance.set_font(self.font.0.as_str(), self.font.1);
        self.metrics = self.instance.text_metrics();
    }

    fn scale_fit(&mut self, w: Option<f32>, h: Option<f32>) {
        let mut bigger = false;
        loop {
            self.update_glyphs();
            let width_bigger = w.map(|w| {
                let tw = self.full_width().unwrap_or(0.0);
                tw > w
            });
            let height_bigger = h.map(|h| {
                let th = self.instance.text_metrics().line_height;
                th > h
            });
            if width_bigger.unwrap_or(false) || height_bigger.unwrap_or(false) {
                bigger = true;
                self.font.1 -= 1.0;
            } else if bigger {
                break
            } else {
                bigger = false;
                self.font.1 += 1.0;
            }
        }
    }

    pub fn display_range_offset(&self) -> Option<f32> {
        self.range.as_ref().map(|range| self.get_glyph(range.start).0)
    }

    pub fn range_bounds(&self, range: Option<Range<usize>>) -> (f32, f32, f32, f32) {
        let slice = self.glyphs_slice();
        let (px, py) = self.pos;
        if slice.is_empty() {
            return (px, py, 0.0, self.metrics.line_height)
        }
        if let Some(range) = range {
            let (smin, _) = self.get_glyph(range.start);
            let (_, emax) = self.get_glyph(range.end);
            (smin, py, emax - smin, self.metrics.line_height)
        } else {
            slice.first().zip(slice.last())
                .map(|(first, last)| {
                    (first.minx, py, last.maxx - first.minx, self.metrics.line_height)
                }).unwrap_or((px, py, 0.0, self.metrics.line_height))
        }
    }

    fn cut_fit(&mut self, width: f32, height: f32, scale: bool) {
        if scale {
            self.scale_fit(None, Some(height));
        }
        let mut range = self.range.clone().unwrap_or(0..self.len());
        range.start = self.offset.min(range.start).min(self.len()).max(0);
        range.end = self.offset.max(range.end).min(self.len()).max(0);
        while range.start < range.end {
            let (.., w, h) = self.range_bounds(Some(range.clone()));
            if w <= width { break }
            let left_dist = self.offset - range.start;
            let right_dist = range.end - range.start;
            if left_dist > right_dist {
                range.start += 1;
            } else {
                range.end -= 1;
            }
        }

        let mut changed = true;
        while changed {
            changed = false;

            if range.start > 0 {
                let (.., w, h) = self.range_bounds(Some((range.start - 1)..range.end));
                if w <= width {
                    range.start -= 1;
                    changed = true;
                }
            }
            if range.end < self.len() {
                let (.., w, h) = self.range_bounds(Some(range.start..(range.end + 1)));
                if w <= width {
                    range.end += 1;
                    changed = true;
                }
            }
        }

        self.range = Some(range);
    }

    fn update_fit(&mut self) {
        let (w, h) = match self.fit_size {
            Some((w, h)) => (w, h),
            None => return
        };

        match self.fit_type {
            TextFitType::Scale => {
                self.scale_fit(Some(w), Some(h));
            }
            TextFitType::RangeAndScale => {
                self.cut_fit(w, h, true);
            }
            TextFitType::Range => {
                self.cut_fit(w, h, false);
            }
        }
    }

    pub fn update(&mut self) {
        self.update_fit();
    }

    pub fn draw(&self, instance: &NvgInstance) {
        let (x, y) = self.pos;
        let (font, font_size) = &self.font;
        instance.set_font(font, *font_size);
        let metrics = instance.text_metrics();
        instance.draw_text((x, y + metrics.descender), self.range.as_ref()
            .map(|range| &self.text[range.clone()]).unwrap_or(&self.text));
    }

}

#[derive(Clone, Debug)]
pub struct TextOld {
    font: String,
    font_size: f32,

    data: String,
    glyph_positions: Vec<NVGglyphPosition>,
    matrics: TextMatrics,
    dirty: bool,
    fit: bool,
    pub range: Option<Range<usize>>,
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

impl TextOld {
    
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        if self.glyph_positions.is_empty() {
            return (self.x, self.y, 0.0, self.matrics.line_height);
        }
        let glyphs = if let Some(range) = &self.range {
            &self.glyph_positions[range.clone()]
        } else {
            &self.glyph_positions
        };
        let w = glyphs.last()
            .zip(glyphs.first())
            .map(|(last, first)| {
                let min = last.minx.min(first.minx);
                let max = last.maxx.max(first.maxx);
                max - min
            })
            .unwrap_or(0.0);
        (self.x, self.y, w, self.matrics.line_height)
    }

    pub fn bounds_absolute(&self) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.bounds();
        (x, y, x + w, y + h)
    }

    pub fn char_bounds(&self, index: usize) -> Option<(f32, f32, f32, f32)> {
        if index >= self.glyph_positions.len() {
            return self.glyph_positions.last().map(|glyph| {
                (self.x + glyph.maxx, self.y, 0.0, self.matrics.line_height)
            });
        }
        let max_index = self.glyph_positions.len().max(1) - 1;
        let index = index.max(0).min(max_index);
        self.glyph_positions.get(index.min(max_index)).map(|glyph| {
            (self.x + glyph.minx, self.y, glyph.maxx - glyph.minx, self.matrics.line_height)
        })
    }

    pub fn char_range_bounds(&self, range: Range<usize>) -> (f32, f32, f32, f32) {
        let x_off = self.range.as_ref()
            .take_if(|other| other.start <= range.start)
            .and_then(|range| self.char_bounds(range.start))
            .map(|(x, ..)| x - self.x)
            .unwrap_or(0.0);
        let (x0, y0, xf0, yf0) = self.char_bounds_absolute(range.start);
        let (x, y, xf, yf) = self.char_bounds_absolute(range.end);
        let w = xf - x0;
        let h = yf - y0;
        (x0 - x_off, y0, w, h)
    }

    pub fn char_range_bounds_absolute(&self, range: Range<usize>) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.char_range_bounds(range);
        (x, y, x + w, y + h)
    }

    pub fn char_bounds_absolute(&self, index: usize) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.char_bounds(index).unwrap_or((self.x, self.y, 0.0, self.matrics.line_height));
        (x, y, x + w, y + h)
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
    
    pub fn set_str(&mut self, string: &str) {
        self.data.clear();
        self.data.extend(string.chars());
        self.glyph_positions.clear();
        self.dirty = true;
    }
    
    pub fn as_str(&self) -> &str {
        self.data.as_str()
    }

    pub fn to_ptrs(&self) -> (*const c_char, *const c_char) {
        unsafe {
            let mut begin = self.data.as_ptr();
            let mut end = begin.add(self.data.len());
            if let Some(range) = &self.range {
                begin = begin.add(range.start);
                end = begin.add(range.len());
            }
            (begin as *const c_char, end as *const c_char)
        }
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
            range: None,
            x: self.x,
            y: self.y,
        })
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }
    
}

#[derive(Debug, Clone)]
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

impl NvgInstance {
    pub fn new() -> NvgInstance {
        unsafe {
            let inside = NvgContextInside::new();
            let context = inside.0;
            let cell = Rc::new(RefCell::new(inside));
            NvgInstance {
                cell,
                context,
                size: (0.0, 0.0),
            }
        }
    }

    pub fn load_font(&mut self, name: &str, path: &str) {
        unsafe {
            let name = CString::from_str(name).unwrap();
            let path = CString::from_str(path).unwrap();
            nvgCreateFont(self.context, name.as_ptr(), path.as_ptr());
        }
    }

    pub fn set_font(&self, name: &str, size: f32) {
        unsafe {
            let name = CString::from_str(name).unwrap();
            let context = self.context;
            nvgFontFace(context, name.as_ptr());
            nvgFontSize(self.context, size);
        }
    }

    pub fn draw_text_old(&mut self, text: &TextOld) {
        unsafe {
            let (ptr, end) = text.to_ptrs();
            self.set_font(text.font.as_str(), text.font_size);
            let mut metrics = self.text_metrics();
            nvgText(self.context, text.x, self.invert_y(text.y + metrics.descender, 0.0), ptr, end);
        }
    }

    pub fn frame<F>(&mut self, size: (f32, f32), draw: F) where F: FnOnce(&mut NvgInstance) {
        unsafe {
            nvgBeginFrame(self.context, size.0, size.1, 1.0);
            self.size = size;
            draw(self);
            nvgEndFrame(self.context);
        }
    }

    pub fn set_size(&mut self, size: (f32, f32)) {
        self.size = size;
    }

    pub fn begin_frame(&mut self, size: (f32, f32)) {
        unsafe {
            nvgBeginFrame(self.context, size.0, size.1, 1.0);
            self.size = size;
        }
    }

    pub fn end_frame(&mut self) {
        unsafe {
            nvgEndFrame(self.context);
        }
    }
    
    pub fn begin_path(&mut self) {
        unsafe {
            nvgBeginPath(self.context);
        }
    }

    pub fn text_glyph_positions(&self, position: (f32, f32), text: &str, dest: &mut [NVGglyphPosition]) {
        unsafe {
            let (x, y) = position;
            let (begin, end) = string_to_ptr_end(text);
            nvgTextGlyphPositions(self.context, x, y, begin, end, dest.as_mut_ptr(), dest.len() as i32);
        }
    }

    pub fn text_metrics(&self) -> TextMatrics {
        unsafe {
            let mut metrics = TextMatrics::empty();
            nvgTextMetrics(self.context, &mut metrics.ascender, &mut metrics.descender, &mut metrics.line_height);
            metrics.ascender = metrics.ascender.abs();
            metrics.descender = metrics.descender.abs();
            metrics.line_height = metrics.line_height.abs();
            metrics
        }
    }

    pub fn draw_text(&self, position: (f32, f32), text: &str) {
        unsafe {
            let (x, y) = position;
            let (begin, end) = string_to_ptr_end(text);
            nvgText(self.context, x, self.invert_y(y, 0.0), begin, end);
        }
    }
    
    pub fn update_text(&mut self, text: &mut TextOld) {
        self.set_font(text.font.as_str(), text.font_size);
        text.matrics = self.text_metrics();
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

    pub fn fit_text(&mut self, text: &mut TextOld, pw: f32, ph: f32) {
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

    pub fn text(&mut self, text: &str, font: &str, font_size: f32) -> TextOld {
        self.set_font(font, font_size);
        let mut matrics = self.text_metrics();
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
            TextOld {
                font: font.to_string(),
                font_size,
                data: text.to_string(),
                glyph_positions,
                matrics,
                dirty: true,
                fit: false,
                range: None,
                x: 0.0,
                y: 0.0
            }
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
}