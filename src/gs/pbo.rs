use std::ptr::null;

pub struct PixelBuffer {
    id: gl::types::GLuint,
    immutable: bool,
    size: usize,
    mapped_ptr: Option<*mut u8>,
}

impl PixelBuffer {
    pub fn allocatePersistent(size: usize, flags: Option<u32>) -> Result<PixelBuffer, String> {
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
}

impl Drop for PixelBuffer {
    fn drop(&mut self) {
        unsafe {
            gl::BindBuffer(gl::PIXEL_UNPACK_BUFFER, self.id);
            if let Some(ptr) = self.mapped_ptr {
                gl::UnmapBuffer(gl::PIXEL_UNPACK_BUFFER);
            }
            gl::BindBuffer(gl::PIXEL_UNPACK_BUFFER, 0);
            gl::DeleteBuffers(1, &self.id);
        }
    }
}