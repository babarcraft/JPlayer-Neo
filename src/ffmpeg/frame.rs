use ffmpeg_sys_next::{av_frame_alloc, av_frame_clone, av_frame_copy_props, av_frame_free, av_frame_unref, av_hwframe_transfer_data, avcodec_receive_frame, AVCodecContext, AVFrame, AVPixelFormat, AVERROR, EAGAIN, EOF};
use crate::ffmpeg::decode::DecoderResult;
use crate::ffmpeg::error::Error;
use crate::ffmpeg::input::Stream;

pub struct Frame {
    pointer: *mut AVFrame,
    pub serial: Option<u32>,
    pub pts: Option<f64>,
    pub duration: Option<f64>,
}

unsafe impl Send for Frame {}

impl Frame {
    pub fn new() -> Self {
        unsafe {
            let frame = av_frame_alloc();
            Self {
                pointer: frame,
                serial: None,
                pts: None,
                duration: None,
            }
        }
    }

    pub(super) fn receive_frame_from_decoder(&mut self, serial: Option<u32>, stream: &Stream, context: *mut AVCodecContext) -> DecoderResult {
        unsafe {
            let result = avcodec_receive_frame(context, self.pointer);
            if result < 0 {
                if result == AVERROR(EAGAIN) || result == AVERROR(EOF) {
                    DecoderResult::NeedsInput
                } else {
                    DecoderResult::Error(Error::from_code(result))
                }
            } else {
                self.update_data(serial, stream);

                DecoderResult::FrameReceived
            }
        }
    }

    fn update_data(&mut self, serial: Option<u32>, stream: &Stream) {
        unsafe {
            self.serial = serial;
            self.pts = Some((*self.pointer).pts as f64 * stream.timebase);
            self.duration = Some((*self.pointer).duration as f64 * stream.timebase);
        }
    }

    pub fn transfer_hw_data_to(&mut self, other: &mut Frame, stream: &Stream) -> Result<(), Error> {
        unsafe {
            let result = av_hwframe_transfer_data(other.pointer, self.pointer, 0);
            if result < 0 {
                return Err(Error::from_code(result))
            }
            let result = av_frame_copy_props(other.pointer, self.pointer);
            if result < 0 {
                return Err(Error::from_code(result))
            }
            self.unref();
            other.update_data(other.serial, stream);
            Ok(())
        }
    }

    pub fn pixel_format(&self) -> Option<AVPixelFormat> {
        unsafe {
            std::mem::transmute((*self.pointer).format)
        }
    }
    
    pub fn dimensions(&self) -> (usize, usize) {
        unsafe {
            ((*self.pointer).width as usize, (*self.pointer).height as usize)
        }
    }
    
    pub fn width(&self) -> usize { 
        unsafe {
            (*self.pointer).width as usize
        }
    }
    
    pub fn height(&self) -> usize {
        unsafe {
            (*self.pointer).height as usize
        }
    }

    pub fn plane(&self, num: usize) -> &[u8] {
        unsafe {
            let line_size = (*self.pointer).linesize[num] as usize;
            let data = (*self.pointer).data[num];
            if data.is_null() {
                panic!("Invalid access of frame data! Plane {num} not found!")
            }
            let height = (*self.pointer).height as usize;
            std::slice::from_raw_parts(data, line_size * height)
        }
    }
    
    pub fn plane_stride(&self, num: usize) -> usize {
        unsafe {
            (*self.pointer).linesize[num] as usize
        }
    }

    pub fn unref(&mut self) {
        unsafe {
            av_frame_unref(self.pointer);
        }
    }
}

impl Clone for Frame {
    fn clone(&self) -> Self {
        unsafe {
            let frame = av_frame_clone(self.pointer);
            Self {
                pointer: frame,
                serial: self.serial,
                pts: self.pts,
                duration: self.duration,
            }
        }
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        unsafe {
            av_frame_free(&mut self.pointer);
        }
    }
}