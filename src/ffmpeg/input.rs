use crate::ffmpeg;
use crate::ffmpeg::error::Error;
use ffmpeg_sys_next::av_dict_set;
use ffmpeg_sys_next::avformat_alloc_context;
use ffmpeg_sys_next::avformat_close_input;
use ffmpeg_sys_next::avformat_open_input;
use ffmpeg_sys_next::AVDictionary;
use ffmpeg_sys_next::AVFormatContext;
use ffmpeg_sys_next::AVPacket;
use std::collections::HashMap;
use std::ffi::CString;
use std::str::FromStr;
use std::sync::atomic::AtomicUsize;

static INPUT_ID: AtomicUsize = AtomicUsize::new(0);

struct Input {
    context: *mut AVFormatContext,
    pub serial: u32,
    pub id: usize
}

impl Input {
    pub fn open(path: &str, options: HashMap<String, String>) -> Result<Self, ffmpeg::error::Error> {
        unsafe {
            let mut context = avformat_alloc_context();
            let mut options_dict: *mut AVDictionary = std::ptr::null_mut();

            for (key, value) in options.iter()
                .map(|(key, value)| (CString::from_str(key.as_str()).unwrap(), CString::from_str(value.as_str()).unwrap())) {
                av_dict_set(&mut options_dict as *mut *mut AVDictionary, key.as_ptr(), value.as_ptr(), 0);
            }

            let path_str = CString::from_str(path).unwrap();
            let result = avformat_open_input(
                &mut context as *mut *mut AVFormatContext,
                path_str.as_ptr(),
                std::ptr::null(),
                &mut options_dict as *mut *mut AVDictionary
            );

            if result < 0 {
                return Err(Error::from_code(result));
            }

            Ok(Input {
                context,
                serial: 0,
                id: INPUT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            })
        }
    }

    pub fn read_packet(&mut self) -> Result<AVPacket, Error> {
        unsafe {
            todo!()
        }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        unsafe {
            avformat_close_input(&mut self.context as *mut *mut AVFormatContext);
        }
    }
}