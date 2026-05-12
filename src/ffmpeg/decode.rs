use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::null_mut;
use ffmpeg_sys_next::{av_hwdevice_ctx_create, avcodec_alloc_context3, avcodec_find_decoder, avcodec_free_context, avcodec_get_hw_config, avcodec_open2, avcodec_parameters_to_context, avcodec_send_packet, AVBufferRef, AVCodec, AVCodecContext, AVCodecHWConfig, AVHWDeviceType, AVPixelFormat, AVERROR_DECODER_NOT_FOUND, AVERROR_EOF, AVERROR_UNKNOWN};
use wgpu::hal::DynOpenDevice;
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
    preferred_pix_format: Box<AVPixelFormat>,
    hardware_frame: Option<Frame>,
    pub serial: Option<u32>,
    pub stream: Stream,
}

unsafe extern "C" fn get_format_callback(context: *mut AVCodecContext, format: *const AVPixelFormat) -> AVPixelFormat {
    unsafe {
        let preferred = (*context).opaque as *const AVPixelFormat;
        let mut current = format;
        while *current != AVPixelFormat::AV_PIX_FMT_NONE {
            if *current == *preferred {
                return *current;
            }

            current = current.offset(1);
        }

        AVPixelFormat::AV_PIX_FMT_NONE
    }
}

fn get_hardware_pix_format(codec: *const AVCodec) -> Option<(AVHWDeviceType, AVPixelFormat)> {
    unsafe {
        let mut i = 0;
        loop {
            let config = avcodec_get_hw_config(codec, i);
            if config.is_null() {
                return None;
            }
            return Some(((*config).device_type, (*config).pix_fmt));
            i = i + 1;
        }
    }
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

            let mut preferred_pix_format = Box::new(AVPixelFormat::AV_PIX_FMT_NONE);
            let mut hardware_download_frame: Option<Frame> = None;

            if let Some((device_type, pixel_format)) = get_hardware_pix_format(codec) {
                let mut device_context: *mut AVBufferRef = std::ptr::null_mut();
                let result = av_hwdevice_ctx_create(&mut device_context, device_type, std::ptr::null(), std::ptr::null_mut(), 0);
                if result < 0 {
                    println!("Failed to create hardware device context! ({:?})", Error::from_code(result));
                } else {
                    *preferred_pix_format = pixel_format;
                    (*context).get_format = Some(get_format_callback);
                    (*context).hw_device_ctx = device_context;
                    (*context).opaque = preferred_pix_format.as_mut() as *mut _ as *mut c_void;
                    hardware_download_frame = Some(Frame::new());
                }
            }

            let result = avcodec_open2(context, codec, &mut convert_options(options));
            if result < 0 {
                return Err(Error::from_code(result))
            }

            Ok(Decoder {
                context,
                serial: None,
                preferred_pix_format,
                hardware_frame: hardware_download_frame,
                stream,
            })
        }
    }

    pub fn receive_frame(&mut self, frame: &mut Frame) -> DecoderResult {
        if let Some(hardware_frame) = &mut self.hardware_frame {
            match hardware_frame.receive_frame_from_decoder(self.serial, &self.stream, self.context) {
                DecoderResult::FrameReceived => {
                    if let Err(error) = hardware_frame.transfer_hw_data_to(frame, &self.stream) {
                        DecoderResult::Error(error)
                    } else {
                        DecoderResult::FrameReceived
                    }
                }
                DecoderResult::NeedsInput => DecoderResult::NeedsInput,
                DecoderResult::Error(error) => DecoderResult::Error(error),
            }
        } else {
            frame.receive_frame_from_decoder(self.serial, &self.stream, self.context)
        }
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