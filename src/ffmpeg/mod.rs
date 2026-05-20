use std::ffi::c_void;
use ffmpeg_sys_next::av_malloc;

pub mod input;
pub mod format;
pub mod decode;
pub mod packet;
pub mod frame;
pub mod error;
pub mod utils;