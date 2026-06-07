use crate::ffmpeg;
use crate::ffmpeg::error::Error;
use ffmpeg_sys_next::{av_dict_set, av_dump_format, av_q2d, av_read_frame, av_seek_frame, avcodec_parameters_alloc, avcodec_parameters_copy, avcodec_parameters_free, avformat_find_stream_info, avformat_flush, avformat_seek_file, avio_seek, AVCodecParameters, AVIOInterruptCB, AVMediaType, AVRational, AVStream, AVERROR_EOF, AVSEEK_FLAG_BACKWARD, AV_TIME_BASE, SEEK_SET};
use ffmpeg_sys_next::avformat_alloc_context;
use ffmpeg_sys_next::avformat_close_input;
use ffmpeg_sys_next::avformat_open_input;
use ffmpeg_sys_next::AVDictionary;
use ffmpeg_sys_next::AVFormatContext;
use ffmpeg_sys_next::AVPacket;
use std::collections::HashMap;
use std::ffi::{c_int, c_void, CString};
use std::ptr::null;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::SystemTime;
use crate::ffmpeg::packet::Packet;
use crate::ffmpeg::utils::{convert_options, convert_options_iter};
use crate::player::clock::{AtomicF64, AtomicInstant};

static INPUT_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, PartialEq)]
pub enum StreamType {
    Video, Audio, Data, Other
}

impl TryFrom<AVMediaType> for StreamType {
    type Error = ();

    fn try_from(value: AVMediaType) -> Result<Self, Self::Error> {
        match value {
            AVMediaType::AVMEDIA_TYPE_VIDEO => Ok(StreamType::Video),
            AVMediaType::AVMEDIA_TYPE_AUDIO => Ok(StreamType::Audio),
            AVMediaType::AVMEDIA_TYPE_DATA => Ok(StreamType::Data),
            _ => Err(())
        }
    }
}

pub struct Stream {
    pub timebase: f64,
    pub start_time: f64,
    pub index: i32,
    pub stream_type: StreamType,
    pub parameters: *mut AVCodecParameters
}

unsafe impl Send for Stream {}

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
                stream_type: StreamType::try_from((*((*other).codecpar)).codec_type).unwrap(),
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
                stream_type: self.stream_type.clone(),
                parameters
            }
        }
    }
}

unsafe extern "C" fn interrupt_callback(context: *mut c_void) -> c_int {
    unsafe {
        let context = context as *const InterruptContext;
        let interrupt = &(*context).interrupt;
        if interrupt.load(Ordering::SeqCst) {
            interrupt.store(false, Ordering::SeqCst);
            1
        } else {
            0
        }
    }
}

#[derive(Clone)]
pub struct InterruptContext {
    interrupt: Arc<AtomicBool>,
    // timeout_secs: Arc<AtomicF64>,
    // timeout_begin: Arc<AtomicInstant>
}

impl InterruptContext {
    pub fn new() -> Box<InterruptContext> {
        Box::new(InterruptContext {
            interrupt: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn interrupt(&self) {
        self.interrupt.store(true, Ordering::SeqCst);
    }
}

pub struct Input {
    context: *mut AVFormatContext,
    pub interrupt_context: Box<InterruptContext>,
    pub options: Vec<(String, String)>,
    pub path: String,

    pub streams: Vec<Stream>,
    pub serial: u32,
    pub id: usize,
    pub read_error: bool,
    after_seek: bool
}

unsafe impl Send for Input {}

impl Input {
    pub fn build_http_headers<'a>(headers: &[(&'a str, &'a str)]) -> String {
        let mut result = String::new();
        for (name, value) in headers {
            result.push_str(format!("{}: {}\r\n", name, value).as_str());
        }
        result
    }

    pub fn open(path: &str, options: &[(&str, &str)]) -> Result<Self, Error> {
        unsafe {
            let mut context = avformat_alloc_context();
            let mut interrupt_context = InterruptContext::new();
            (*context).interrupt_callback = AVIOInterruptCB {
                callback: Some(interrupt_callback),
                opaque: interrupt_context.as_mut() as *mut _ as *mut c_void,
            };
            let path_str = CString::from_str(path).unwrap();
            let result = avformat_open_input(
                &mut context as *mut *mut AVFormatContext,
                path_str.as_ptr(),
                std::ptr::null(),
                &mut convert_options(options),
            );
            if result < 0 {
                return Err(Error::from_code(result));
            }
            
            let result = avformat_find_stream_info(context, std::ptr::null_mut());
            if result < 0 {
                return Err(Error::from_code(result));
            }

            av_dump_format(context, 0, null(), 0);

            let streams = std::slice::from_raw_parts((*context).streams, (*context).nb_streams as usize)
                .iter().map(|stream| Stream::from_stream(*stream)).collect();

            Ok(Input {
                context,
                serial: 0,
                interrupt_context,
                options: options.iter().map(|&(k, v)| (k.to_string(), v.to_string())).collect(),
                path: path.to_string(),
                id: INPUT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                read_error: false,
                after_seek: false,
                streams
            })
        }
    }

    pub fn flush(&mut self) {
        unsafe {
            avformat_flush(self.context);
        }
    }

    pub fn restart(&mut self) -> Result<(), Error> {
        unsafe {
            avformat_close_input(&mut self.context);
            self.context = avformat_alloc_context();
            let path_str = CString::from_str(self.path.as_str()).unwrap();
            let result = avformat_open_input(
                &mut self.context as *mut *mut AVFormatContext,
                path_str.as_ptr(),
                std::ptr::null(),
                &mut convert_options_iter(&mut self.options.iter().map(|(key, value)| (key.as_str(), value.as_str()))),
            );
            if result < 0 {
                return Err(Error::from_code(result));
            }

            let result = avformat_find_stream_info(self.context, std::ptr::null_mut());
            if result < 0 {
                return Err(Error::from_code(result));
            }

            av_dump_format(self.context, 0, null(), 0);

            self.streams = std::slice::from_raw_parts((*self.context).streams, (*self.context).nb_streams as usize)
                .iter().map(|stream| Stream::from_stream(*stream)).collect();
            self.read_error = false;
            self.serial = 0;
            Ok(())
        }
    }
    
    pub fn duration(&self) -> f64 {
        unsafe {
            (*self.context).duration as f64 / AV_TIME_BASE as f64
        }
    }

    pub fn read_packet(&mut self) -> Result<Packet, Error> {
        let mut packet = Packet::new(self.serial, self.id);
        if let Err(error) = packet.read_from(self.context) {
            self.restart()?;
            self.read_error = true;
            return Err(error);
        }
        Ok(packet)
    }

    pub fn seek(&mut self, min: f64, max: f64, stream_index: Option<i32>) -> Result<(), Error> {
        if self.read_error {
            self.restart()?;
        }
        let (min_ts, max_ts, index) = stream_index.and_then(|index| {
            let base = self.streams[index as usize].timebase;
            Some(((min / base) as i64, (max / base) as i64, index))
        }).unwrap_or(((min * AV_TIME_BASE as f64) as i64, (max * ffmpeg_sys_next::AV_TIME_BASE as f64) as i64, -1));
        unsafe {
            let result = avformat_seek_file(
                self.context,
                index,
                min_ts,
                max_ts,
                max_ts,
                AVSEEK_FLAG_BACKWARD
            );
            if result < 0 {
                return Err(Error::from_code(result));
            }
            self.read_error = false;
            self.serial += 1;
            Ok(())
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