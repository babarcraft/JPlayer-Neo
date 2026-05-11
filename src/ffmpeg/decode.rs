use std::collections::HashMap;
use std::ffi::c_void;
use ffmpeg_sys_next::{avcodec_alloc_context3, avcodec_find_decoder, avcodec_free_context, avcodec_open2, avcodec_parameters_to_context, avcodec_send_packet, AVCodecContext, AVPixelFormat, AVERROR_DECODER_NOT_FOUND, AVERROR_EOF, AVERROR_UNKNOWN};
use crate::ffmpeg::error::Error;
use crate::ffmpeg::frame::Frame;
use crate::ffmpeg::input::Stream;
use crate::ffmpeg::packet::Packet;
use crate::ffmpeg::utils::convert_options;

pub enum DecoderResult {
    FrameReceived,
    NeedsInput,
    Error(Error),
}

pub struct Decoder {
    context: *mut AVCodecContext,
    pub serial: Option<u32>,
    pub stream: Stream,
}

unsafe extern "C" fn get_format_callback(context: *mut AVCodecContext, format: *const AVPixelFormat) -> AVPixelFormat {
    let dest = (*context).opaque as *mut AVPixelFormat;
    todo!()
}
impl Decoder {
    pub fn new(stream: Stream, options: Vec<(&str, &str)>) -> Result<Decoder, Error> {
        unsafe {
            let parameters = stream.parameters;
            let codec = avcodec_find_decoder((*parameters).codec_id);
            if codec.is_null() {
                return Err(Error::from_code(AVERROR_DECODER_NOT_FOUND))
            }
            let context = avcodec_alloc_context3(codec);
            if context.is_null() {
                return Err(Error::from_code(AVERROR_UNKNOWN))
            }

            let result = avcodec_parameters_to_context(context, parameters);
            if result < 0 {
                return Err(Error::from_code(result))
            }

            let result = avcodec_open2(context, codec, &mut convert_options(options));
            if result < 0 {
                return Err(Error::from_code(result))
            }

            Ok(Decoder {
                context,
                serial: None,
                stream,
            })
        }
    }

    pub fn receive_frame(&mut self, frame: &mut Frame) -> DecoderResult {
        frame.receive_frame_from_decoder(self.serial, &self.stream, self.context)
    }

    pub fn send_packet(&mut self, packet: Packet) -> Result<(), Error> {
        unsafe {
            let result = avcodec_send_packet(self.context, packet.pointer);
            if result < 0 {
                Err(Error::from_code(result))
            } else {
                Ok(())
            }
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            avcodec_free_context(&mut self.context);
        }
    }
}