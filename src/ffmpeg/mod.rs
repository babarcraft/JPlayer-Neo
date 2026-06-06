use std::ffi::c_void;
use ffmpeg_sys_next::{av_free, av_gettime_relative, av_malloc};

pub mod input;
pub mod format;
pub mod decode;
pub mod packet;
pub mod frame;
pub mod error;
pub mod utils;

pub fn current_time() -> u64 {
    unsafe {
        av_gettime_relative().max(0) as u64
    }
}

pub struct Buffer {
    ptr: *mut u8,
    size: usize,
}
