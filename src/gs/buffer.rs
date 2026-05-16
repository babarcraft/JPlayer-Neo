use std::ptr::null;
use gl::types::{GLint, GLuint};
use crate::gs::texture::{InternalFormat, Texture};

pub struct PixelBuffer {
    id: gl::types::GLuint,
    immutable: bool,
    size: usize,
    mapped_ptr: Option<*mut u8>,
}

impl PixelBuffer {
    pub fn allocate_persistent(size: usize, flags: Option<u32>) -> Result<PixelBuffer, String> {
        unsafe {
            let mut id = 0;
            gl::GenBuffers(1, &mut id);
            let flags = flags.or(Some(gl::MAP_PERSISTENT_BIT | gl::MAP_COHERENT_BIT | gl::MAP_WRITE_BIT)).unwrap();
            gl::BindBuffer(gl::PIXEL_UNPACK_BUFFER, id);
            gl::BufferStorage(gl::PIXEL_UNPACK_BUFFER, size as gl::types::GLsizeiptr, null(), flags);
            let ptr = gl::MapBufferRange(gl::PIXEL_UNPACK_BUFFER, 0, size as gl::types::GLsizeiptr, flags);
            if ptr.is_null() {
                return Err("Unable to map!".into())
            }
            Ok(PixelBuffer {
                id,
                immutable: true,
                size,
                mapped_ptr: Some(ptr as *mut u8),
            })
        }
    }

    pub fn bind(&self) {
        unsafe {
            gl::BindBuffer(gl::PIXEL_UNPACK_BUFFER, self.id);
        }
    }

    pub fn unbind(&self) {
        unsafe {
            gl::BindBuffer(gl::PIXEL_UNPACK_BUFFER, 0);
        }
    }

    pub fn mapped(&mut self) -> Option<&mut [u8]> {
        unsafe {
            self.mapped_ptr.clone().map(|ptr| std::slice::from_raw_parts_mut(ptr, self.size))
        }
    }
}

impl Drop for PixelBuffer {
    fn drop(&mut self) {
        unsafe {
            gl::BindBuffer(gl::PIXEL_UNPACK_BUFFER, self.id);
            if let Some(_) = self.mapped_ptr {
                gl::UnmapBuffer(gl::PIXEL_UNPACK_BUFFER);
            }
            gl::BindBuffer(gl::PIXEL_UNPACK_BUFFER, 0);
            gl::DeleteBuffers(1, &self.id);
        }
    }
}

pub struct FrameBuffer {
    id: GLuint,
    pub texture: Texture,
}

impl FrameBuffer {
    pub fn new() -> FrameBuffer {
        unsafe {
            let mut id = 0;
            gl::GenFramebuffers(1, &mut id);
            FrameBuffer {
                id,
                texture: Texture::new()
            }
        }
    }

    pub fn draw<F>(&mut self, width: u32, height: u32, mut draw: F) where F: FnMut() {
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, self.id);
            self.texture.bind();

            if !self.texture.has_space(width, height, InternalFormat::Rgba(8)) {
                self.texture.upload(None, None, width, height, InternalFormat::Rgba(8));
                self.texture.set_parameters(
                    gl::LINEAR,
                    gl::LINEAR,
                    gl::NEAREST,
                    gl::NEAREST,
                )
            }

            let mut viewport: [GLint; 4] = [0; 4];
            gl::GetIntegerv(gl::VIEWPORT, viewport[..].as_mut_ptr());
            gl::Clear(gl::COLOR_BUFFER_BIT);
            gl::Viewport(0, 0, width as GLint, height as GLint);
            draw();

            self.texture.unbind();
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);

            gl::Viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
        }
    }
}

impl Drop for FrameBuffer {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteFramebuffers(1, &self.id);
        }
    }
}

pub enum LayoutElement {
    Float(usize),
    FloatNormalized(usize),
    Int(usize),
    IntNormalized(usize),
}

pub struct VertexArray {
    id: GLuint,
}

impl Drop for VertexArray {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteVertexArrays(1, &self.id);
        }
    }
}