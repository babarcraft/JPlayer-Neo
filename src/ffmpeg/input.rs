use crate::ffmpeg;
use crate::ffmpeg::error::Error;
use ffmpeg_sys_next::{av_dict_set, av_q2d, av_read_frame, avcodec_parameters_alloc, avcodec_parameters_copy, avcodec_parameters_free, AVCodecParameters, AVRational, AVStream};
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
use crate::ffmpeg::packet::Packet;

static INPUT_ID: AtomicUsize = AtomicUsize::new(0);

pub struct Stream {
    pub timebase: f64,
    pub start_time: f64,
    pub index: i32,
    pub parameters: *mut AVCodecParameters
}

impl Stream {
    fn from_stream(other: *const AVStream) -> Self {
        unsafe {
            let parameters = avcodec_parameters_alloc();
            if parameters.is_null() {
                panic!("avcodec_parameters_alloc failed.");
            }
            avcodec_parameters_copy(parameters, (*other).codecpar);
            let timebase = av_q2d((*other).time_base);
            Self {
                timebase,
                start_time: (*other).start_time as f64 * timebase,
                index: (*other).index,
                parameters
            }
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe {
            avcodec_parameters_free(&mut self.parameters);
        }
    }
}

impl Clone for Stream {
    fn clone(&self) -> Self {
        unsafe {
            let parameters = avcodec_parameters_alloc();
            if parameters.is_null() {
                panic!("avcodec_parameters_alloc failed.");
            }
            avcodec_parameters_copy(parameters, self.parameters);
            Self {
                timebase: self.timebase,
                start_time: self.start_time,
                index: self.index,
                parameters
            }
        }
    }
}

pub struct Input {
    context: *mut AVFormatContext,
    pub streams: Vec<Stream>,
    pub serial: u32,
    pub id: usize
}

impl Input {
    pub fn open(path: &str, options: HashMap<String, String>) -> Result<Self, Error> {
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

            let streams = std::slice::from_raw_parts((*context).streams, (*context).nb_streams as usize)
                .iter().map(|stream| Stream::from_stream(*stream)).collect();

            Ok(Input {
                context,
                serial: 0,
                id: INPUT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                streams
            })
        }
    }

    pub fn read_packet(&mut self) -> Result<Packet, Error> {
        let mut packet = Packet::new(self.serial, self.id);
        if let Some(error) = packet.read_from(self.context) {
            return Err(error);
        }
        Ok(packet)
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        unsafe {
            avformat_close_input(&mut self.context as *mut *mut AVFormatContext);
        }
    }
}