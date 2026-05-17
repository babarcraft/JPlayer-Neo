use std::ptr::null;
use gl::types::{GLenum, GLint, GLsizei, GLuint};
use crate::gs::gl::check_errors;

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum InternalFormat {
    Rgba(u8),
    Rgb(u8),
    Bgra,
    Bgr,
    R(u8),
    Rg(u8),
}

impl InternalFormat {
    pub fn get_texel_size_bytes(&self) -> usize {
        match *self {
            InternalFormat::Rgba(bits) => if bits <= 8 {
                4
            } else if bits <= 16 {
                8
            } else {
                panic!("Unsupported format bits: {bits}")
            },
            InternalFormat::Rgb(bits) => if bits <= 8 {
                3
            } else if bits <= 16 {
                6
            } else {
                panic!("Unsupported format bits: {bits}")
            },
            InternalFormat::Bgr => 3,
            InternalFormat::Bgra => 4,
            InternalFormat::R(bits) => if bits <= 8 {
                1
            } else if bits <= 16 {
                2
            } else {
                panic!("Unsupported format bits: {bits}")
            },
            InternalFormat::Rg(bits) => if bits <= 8 {
                2
            } else if bits <= 16 {
                4
            } else {
                panic!("Unsupported format bits: {bits}")
            }
        }
    }

    pub fn get_internal_format(&self) -> GLint {
        (match *self {
            InternalFormat::Rgba(bits) => if bits <= 8 {
                gl::RGBA8
            } else if bits <= 16 {
                gl::RGBA16
            } else {
                panic!("Unsupported format bits: {bits}")
            },
            InternalFormat::Rgb(bits) => if bits <= 8 {
                gl::RGB8
            } else if bits <= 16 {
                gl::RGB16
            } else {
                panic!("Unsupported format bits: {bits}")
            },
            InternalFormat::Bgr => gl::BGR,
            InternalFormat::Bgra => gl::BGRA,
            InternalFormat::R(bits) => if bits <= 8 {
                gl::R8
            } else if bits <= 16 {
                gl::R16
            } else {
                panic!("Unsupported format bits: {bits}")
            },
            InternalFormat::Rg(bits) => if bits <= 8 {
                gl::RG8
            } else if bits <= 16 {
                gl::RG16
            } else {
                panic!("Unsupported format bits: {bits}")
            }
        }) as GLint
    }

    pub fn get_format(&self) -> GLuint {
        match *self {
            InternalFormat::Rgba(_) => gl::RGBA,
            InternalFormat::Rgb(_) => gl::RGB,
            InternalFormat::Bgr => gl::BGR,
            InternalFormat::Bgra => gl::BGRA,
            InternalFormat::R(_) => gl::RED,
            InternalFormat::Rg(_) => gl::RG,
        }
    }

    pub fn get_type(&self) -> GLenum {
        match *self {
            InternalFormat::Rgba(bits) | InternalFormat::R(bits) | InternalFormat::Rg(bits) | InternalFormat::Rgb(bits) => if bits <= 8 {
                gl::UNSIGNED_BYTE
            } else if bits <= 16 {
                gl::UNSIGNED_SHORT
            } else {
                panic!("Unsupported format bits: {bits}")
            },
            InternalFormat::Bgr | InternalFormat::Bgra => gl::UNSIGNED_BYTE,
        }
    }
}

pub struct Texture {
    pub(super) id: GLuint,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub format: Option<InternalFormat>,
}

impl Texture {
    pub fn new() -> Texture {
        unsafe {
            let mut id: GLuint = 0;
            gl::GenTextures(1, &mut id);
            Texture {
                id,
                width: None,
                height: None,
                format: None,
            }
        }
    }

    pub fn bind(&self, slot: Option<u8>) {
        unsafe {
            if let Some(slot) = slot {
                gl::ActiveTexture(gl::TEXTURE0 + slot as GLuint);
            }
            gl::BindTexture(gl::TEXTURE_2D, self.id);
        }
    }

    pub fn unbind(&self) {
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }
    }

    pub fn has_space(&self, width: u32, height: u32, format: InternalFormat) -> bool {
        let self_width = match self.width {
            Some(width) => width,
            None => return false,
        };
        let self_height = match self.height {
            Some(height) => height,
            None => return false,
        };
        let self_format = match self.format {
            Some(format) => format,
            None => return false,
        };

        width == self_width && height == self_height && format == self_format
    }

    pub fn set_parameters(&self, mag_filter: GLenum, min_filter: GLenum, wrap_s: GLenum, wrap_t: GLenum) {
        unsafe {
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, min_filter as GLint);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, mag_filter as GLint);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, wrap_s as GLint);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, wrap_t as GLint);
            check_errors("Set Texture Parameters", false)
        }
    }

    pub fn upload(&mut self, data: Option<&[u8]>, stride: Option<usize>, width: u32, height: u32, format: InternalFormat) {
        unsafe {
            self.bind(None);
            if let Some(stride) = stride {
                gl::PixelStorei(gl::UNPACK_ROW_LENGTH, (stride / format.get_texel_size_bytes()) as GLint)
            }
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                format.get_internal_format(),
                width as GLsizei,
                height as GLsizei,
                0,
                format.get_format(),
                format.get_type(),
                data.map(|data| data.as_ptr() as *const std::ffi::c_void).unwrap_or(null())
            );
            check_errors("Texture Upload", false);
            self.width = Some(width);
            self.height = Some(height);
            self.format = Some(format);

            if let Some(_) = stride {
                gl::PixelStorei(gl::UNPACK_ROW_LENGTH, 0)
            }
            self.unbind();
        }
    }

    pub fn bind_image(&self, slot: u32) {
        unsafe {
            gl::BindImageTexture(slot, self.id, 0, gl::FALSE, 0, gl::WRITE_ONLY, self.format.unwrap().get_internal_format() as GLenum);
        }
    }

    pub fn upload_partial(&mut self, data: Option<&[u8]>, stride: Option<usize>, ox: u32, oy: u32, width: u32, height: u32) {
        unsafe {
            self.bind(None);
            let format = self.format.expect("Expected allocated texture");
            if let Some(stride) = stride {
                gl::PixelStorei(gl::UNPACK_ROW_LENGTH, (stride / format.get_texel_size_bytes()) as GLint)
            }
            gl::TexSubImage2D(
                gl::TEXTURE_2D,
                0,
                ox as GLsizei,
                oy as GLsizei,
                width as GLsizei,
                height as GLsizei,
                format.get_format(),
                format.get_type(),
                data.map(|data| data.as_ptr() as *const std::ffi::c_void).unwrap_or(null())
            );
            check_errors("Texture Partial Upload", false);

            if let Some(_) = stride {
                gl::PixelStorei(gl::UNPACK_ROW_LENGTH, 0)
            }
            self.unbind();
        }
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteTextures(1, &self.id);
        }
    }
}
