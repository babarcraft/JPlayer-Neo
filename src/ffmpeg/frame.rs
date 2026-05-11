use ffmpeg_sys_next::{av_frame_alloc, av_frame_clone, av_frame_free, av_frame_unref, avcodec_receive_frame, AVCodecContext, AVFrame, AVERROR, EAGAIN, EOF};
use crate::ffmpeg::decode::DecoderResult;
use crate::ffmpeg::error::Error;
use crate::ffmpeg::input::Stream;

pub struct Frame {
    pointer: *mut AVFrame,
    pub serial: Option<u32>,
    pub pts: Option<f64>,
}

impl Frame {
    pub fn new() -> Self {
        unsafe {
            let frame = av_frame_alloc();
            Self {
                pointer: frame,
                serial: None,
                pts: None,
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
                self.serial = serial;
                
                // Set further data like pts, duration etc
                self.pts = Some((*self.pointer).pts as f64 * stream.timebase);
                
                DecoderResult::FrameReceived
            }
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