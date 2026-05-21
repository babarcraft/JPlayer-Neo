use crate::ffmpeg::frame::Frame;
use crate::gs::buffer::PixelBuffer;
use crate::gs::fence::Fence;
use crate::gs::gl::mapped_buffer_barrier;
use crate::gs::shader::Shader;
use crate::gs::texture::{InternalFormat, Texture};

pub struct FrameQueue {
    frames: Vec<Frame>,
    size: usize,
    read_index: usize,
    write_index: usize,
}

impl FrameQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            frames: (0..capacity).map(|_| Frame::new()).collect(),
            size: 0,
            read_index: 0,
            write_index: 0,
        }
    }

    pub fn has_space(&self) -> bool {
        self.size < self.frames.len()
    }

    pub fn queued_num(&self) -> usize {
        self.size
    }

    pub fn peek_write(&mut self) -> Option<&mut Frame> {
        if self.size >= self.frames.len() {
            return None;
        }

        Some(&mut self.frames[self.write_index])
    }

    pub fn push(&mut self) {
        self.write_index = (self.write_index + 1) % self.frames.len();
        self.size += 1;
    }

    pub fn peek_read(&self) -> Option<&Frame> {
        if self.size <= 0 {
            return None;
        }

        let frame = &self.frames[self.read_index];
        Some(frame)
    }
    
    pub fn queued(&self) -> usize {
        self.size
    }

    pub fn pop(&mut self) {
        self.read_index = (self.read_index + 1) % self.frames.len();
        self.size -= 1;
    }
}

#[derive(Clone, Copy)]
pub struct SlotPlaneDescription {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: InternalFormat
}

pub struct UploadSlot {
    pixel_buffers: [Option<PixelBuffer>; 4],
    /// Contains the dimensions for each plane in the following order: width, height, stride
    uploaded_planes_dimensions: [SlotPlaneDescription; 4],
    uploaded_planes: Option<usize>,
    fence: Fence,
}

impl UploadSlot {
    pub fn new() -> UploadSlot {
        UploadSlot {
            pixel_buffers: [const { None } ; 4],
            uploaded_planes_dimensions: [const { SlotPlaneDescription {
                width: 0,
                height: 0,
                stride: 0,
                format: InternalFormat::R(0)
            } }; 4],
            uploaded_planes: None,
            fence: Fence::new(),
        }
    }

    pub fn check_and_allocate(&mut self, frame: &Frame, plane_formats: &[InternalFormat], chroma: Option<usize>) {
        for (plane, format) in plane_formats.iter().enumerate() {
            let chroma = chroma.clone().take_if(|_| plane > 0).unwrap_or(1);

            let stride = frame.plane_stride(plane);
            let height = frame.height() / chroma;
            let width = frame.width() / chroma;
            self.uploaded_planes_dimensions[plane] = SlotPlaneDescription {
                width: width as u32,
                height: height as u32,
                stride,
                format: *format
            };
            let allocate = || PixelBuffer::allocate_persistent(height * stride, None).unwrap();
            let pixel_buffer = self.pixel_buffers[plane].get_or_insert_with(allocate);
            if let Some(mapped) = pixel_buffer.mapped() {
                if mapped.len() != height * stride {
                    self.pixel_buffers[plane].replace(allocate());
                }
            } else {
                self.pixel_buffers[plane].replace(allocate());
            }
        }
    }

    pub fn upload(&mut self, frame: &Frame, plane_formats: &[InternalFormat], chroma: Option<usize>) {
        self.check_and_allocate(frame, plane_formats, chroma);
        for plane in 0..plane_formats.len() {
            let mapped = self.pixel_buffers[plane].as_ref().unwrap().mapped().unwrap();
            mapped.copy_from_slice(frame.plane(plane, chroma));
            mapped_buffer_barrier();
        }
        self.uploaded_planes = Some(plane_formats.len());
    }

    pub fn upload_to_textures_bind(&self, textures: &[Texture]) {
        for (index, texture) in textures[..self.uploaded_planes.unwrap()].iter().enumerate() {
            let pixel_buffer = self.pixel_buffers[index].as_ref().unwrap();
            let desc = self.uploaded_planes_dimensions[index];
            pixel_buffer.bind();
            texture.bind(Some(index as u8));
            texture.upload_partial(None, Some(desc.stride), 0, 0, desc.width, desc.height);
            pixel_buffer.unbind();
        }
    }
}

pub struct VideoSurface {
    upload_slots: [UploadSlot; 3],
    size: u8,
    write_index: u8,
    read_index: u8,
    upload_textures: [Texture; 3],
    pub output_texture: Texture,
    compute_shader: Option<Shader>,
    shader_planes: usize
}

impl VideoSurface {
    pub fn new() -> VideoSurface {
        VideoSurface {
            upload_slots: [
                UploadSlot::new(),
                UploadSlot::new(),
                UploadSlot::new(),
            ],
            upload_textures: [
                Texture::new(),
                Texture::new(),
                Texture::new(),
            ],
            output_texture: Texture::new(),
            compute_shader: None,
            shader_planes: 0,
            size: 0,
            write_index: 0,
            read_index: 0,
        }
    }

    pub fn ensure_compute_shader(&mut self) {
        let slot = &self.upload_slots[self.read_index as usize];
        let planes = match slot.uploaded_planes {
            Some(planes) => planes,
            None => return
        };
        if self.shader_planes != planes {
            let shader = match planes {
                2 => Shader::compile_compute(include_str!("../res/nv12.glsl")).ok(),
                _ => None
            };
            if shader.is_some() {
                self.shader_planes = planes;
            }
            self.compute_shader = shader;
        }
    }

    pub fn ensure_texture_size(&mut self) {
        let slot = &self.upload_slots[self.read_index as usize];
        let planes = match slot.uploaded_planes {
            Some(planes) => planes,
            None => return
        };
        let desc = slot.uploaded_planes_dimensions[0];
        if !self.output_texture.has_space(desc.width, desc.height, InternalFormat::Rgba(8)) {
            self.output_texture.bind(Some(0));
            self.output_texture.upload(None, None, desc.width, desc.height, InternalFormat::Rgba(8));
            self.output_texture.set_parameters(
                gl::LINEAR,
                gl::LINEAR,
                gl::CLAMP_TO_EDGE,
                gl::CLAMP_TO_EDGE,
            );
            self.output_texture.unbind();
        }
        for plane in 0..planes {
            let desc = slot.uploaded_planes_dimensions[plane];
            let texture = &mut self.upload_textures[plane];
            if !texture.has_space(desc.width, desc.height, desc.format) {
                texture.bind(Some(0));
                texture.upload(None, None, desc.width, desc.height, desc.format);
                texture.set_parameters(
                    gl::LINEAR,
                    gl::LINEAR,
                    gl::CLAMP_TO_EDGE,
                    gl::CLAMP_TO_EDGE,
                );
                texture.unbind();
            }
        }
    }

    pub fn can_upload(&self) -> bool {
        self.size < self.upload_slots.len() as u8
    }

    pub fn upload(&mut self, frame: &Frame, plane_formats: &[InternalFormat], chroma: Option<usize>) {
        if !self.can_upload() {
            return;
        }

        {
            let slot = &mut self.upload_slots[self.write_index as usize];
            slot.upload(frame, plane_formats, chroma);
            self.ensure_compute_shader();
            self.ensure_texture_size();
        }


        self.write_index = (self.write_index + 1) % self.upload_slots.len() as u8;
        self.size += 1;
    }

    pub fn convert_output(&mut self) {
        if self.size <= 0 {
            return;
        }

        let shader = match &self.compute_shader {
            Some(shader) => shader,
            None => return
        };

        let slot = &mut self.upload_slots[self.read_index as usize];

        if !slot.fence.check_done(None) {
            return;
        }

        shader.bind();
        let desc = slot.uploaded_planes_dimensions[0];
        slot.upload_to_textures_bind(&self.upload_textures);
        self.output_texture.bind_image(slot.uploaded_planes.unwrap() as u32);
        shader.dispatch_compute((desc.width + 15) / 16, (desc.height + 15) / 16, 1);
        shader.image_access_barrier();
        shader.unbind();
        slot.fence.place();

        self.read_index = (self.read_index + 1) % self.upload_slots.len() as u8;
        self.size -= 1;
    }
}