use std::ffi::c_void;
use std::fs::symlink_metadata;
use std::mem;
use std::ptr::{null, null_mut};
use gl::types::{GLenum, GLint, GLsizei, GLsizeiptr, GLuint, GLvoid};
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
            gl::BindBuffer(gl::PIXEL_UNPACK_BUFFER, 0);
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

    pub fn mapped(&self) -> Option<&mut [u8]> {
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
            self.texture.bind(None);

            if !self.texture.has_space(width, height, InternalFormat::Rgba(8)) {
                self.texture.upload(None, None, width, height, InternalFormat::Rgba(8));
                self.texture.set_parameters(
                    gl::LINEAR,
                    gl::LINEAR,
                    gl::NEAREST,
                    gl::NEAREST,
                );
                gl::FramebufferTexture2D(gl::FRAMEBUFFER, gl::COLOR_ATTACHMENT0, gl::TEXTURE_2D, self.texture.id, 0);
                let result = gl::CheckFramebufferStatus(gl::FRAMEBUFFER);
                if result != gl::FRAMEBUFFER_COMPLETE {
                    println!("Failed to attach FrameBuffer to Texture!");
                    return;
                }
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

#[derive(Copy, Clone)]
pub enum LayoutElementType {
    Float, Integer
}

pub enum LayoutElementStep {
    Vertex, Instance
}

impl LayoutElementType {
    pub fn byte_count(&self) -> usize {
        match self {
            LayoutElementType::Float => size_of::<f32>(),
            LayoutElementType::Integer => size_of::<i32>(),
        }
    }
}

impl Into<GLenum> for LayoutElementType {
    fn into(self) -> GLenum {
        match self {
            LayoutElementType::Float => gl::FLOAT,
            LayoutElementType::Integer => gl::INT,
        }
    }
}

pub struct LayoutElement {
    pub(crate) layout_element: LayoutElementType,
    pub(crate) count: usize,
    pub(crate) step: LayoutElementStep,
}

pub struct VertexBuffer {
    id: GLuint,
    pub size: Option<usize>,
}

impl VertexBuffer {
    pub fn new() -> VertexBuffer {
        unsafe {
            let mut id = 0;
            gl::GenBuffers(1, &mut id);
            VertexBuffer {
                id,
                size: None,
            }
        }
    }

    pub fn bind(&self) {
        unsafe {
            gl::BindBuffer(gl::ARRAY_BUFFER, self.id);
        }
    }

    pub fn unbind(&self) {
        unsafe {
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
        }
    }

    pub fn upload_f32(&mut self, data: &[f32]) {
        unsafe {
            self.bind();
            let size = data.len() * mem::size_of::<f32>();
            gl::BufferData(gl::ARRAY_BUFFER, size as GLsizeiptr, data.as_ptr() as *const c_void, gl::STATIC_DRAW);
            self.size = Some(size);
            self.unbind();
        }
    }

}

impl Drop for VertexBuffer {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteBuffers(1, &self.id);
        }
    }
}

pub struct ElementBuffer {
    id: GLuint,
    pub size: Option<usize>,
}

impl ElementBuffer {
    pub fn new() -> ElementBuffer {
        unsafe {
            let mut id = 0;
            gl::GenBuffers(1, &mut id);
            ElementBuffer {
                id,
                size: None,
            }
        }
    }

    pub fn bind(&self) {
        unsafe {
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, self.id);
        }
    }

    pub fn unbind(&self) {
        unsafe {
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, 0);
        }
    }

    pub fn upload_u16(&mut self, data: &[u16]) {
        unsafe {
            self.bind();
            let size = data.len() * mem::size_of::<u16>();
            gl::BufferData(gl::ELEMENT_ARRAY_BUFFER, size as GLsizeiptr, data.as_ptr() as *const c_void, gl::STATIC_DRAW);
            self.size = Some(size);
            self.unbind();
        }
    }
}

impl Drop for ElementBuffer {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteBuffers(1, &self.id);
        }
    }
}

pub struct VertexArray {
    id: GLuint,
    current_index: usize,
}

impl VertexArray {
    pub fn new() -> VertexArray {
        unsafe {
            let mut id = 0;
            gl::GenVertexArrays(1, &mut id);
            VertexArray {
                id,
                current_index: 0,
            }
        }
    }

    pub fn attach_element_buffer(&mut self, buffer: &ElementBuffer) {
        unsafe {
            gl::BindVertexArray(self.id);
            buffer.bind();
            gl::BindVertexArray(0);
            buffer.unbind();
        }
    }

    pub fn attach_vertex_buffer(&mut self, buffer: &VertexBuffer, layout: &[LayoutElement]) {
        unsafe {
            gl::BindVertexArray(self.id);
            buffer.bind();
            let stride = layout.iter()
                .map(|element| element.count * element.layout_element.byte_count())
                .sum::<usize>();
            let mut current_offset = 0;
            for element in layout {
                let size = element.count * element.layout_element.byte_count();
                gl::VertexAttribPointer(
                    self.current_index as GLuint,
                    element.count as GLint,
                    element.layout_element.clone().into(),
                    gl::FALSE,
                    stride as GLsizei,
                    current_offset as *const GLvoid
                );
                gl::EnableVertexAttribArray(self.current_index as GLuint);
                match element.step {
                    LayoutElementStep::Vertex => {
                        gl::VertexAttribDivisor(self.current_index as GLuint, 0);
                    },
                    LayoutElementStep::Instance => {
                        gl::VertexAttribDivisor(self.current_index as GLuint, 1);
                    }
                }

                current_offset += size;
                self.current_index += 1;
            }
            gl::BindVertexArray(0);

            buffer.unbind();
        }
    }

    pub fn draw_indexed(&self, mode: GLenum, count: usize, ty: GLenum) {
        unsafe {
            gl::BindVertexArray(self.id);
            gl::DrawElements(mode, count as GLsizei, ty, std::ptr::null());
            gl::BindVertexArray(0);
        }
    }
}

impl Drop for VertexArray {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteVertexArrays(1, &self.id);
        }
    }
}