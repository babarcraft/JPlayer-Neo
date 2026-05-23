use std::arch::x86_64::_mm256_andnot_si256;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::null_mut;
use cpal::SampleFormat;
use ffmpeg_sys_next::{av_channel_layout_default, av_get_bytes_per_sample, av_hwdevice_ctx_create, av_sample_fmt_is_planar, avcodec_alloc_context3, avcodec_find_decoder, avcodec_flush_buffers, avcodec_free_context, avcodec_get_hw_config, avcodec_open2, avcodec_parameters_to_context, avcodec_send_packet, swr_alloc, swr_alloc_set_opts2, swr_close, swr_convert, swr_free, swr_get_out_samples, swr_init, AVBufferRef, AVChannelLayout, AVChannelLayout__bindgen_ty_1, AVChannelOrder, AVCodec, AVCodecContext, AVCodecHWConfig, AVHWDeviceType, AVPixelFormat, AVSampleFormat, SwrContext, AVERROR_DECODER_NOT_FOUND, AVERROR_EOF, AVERROR_UNKNOWN};
use crate::ffmpeg::error::Error;
use crate::ffmpeg::frame::{AudioFrame, Frame};
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
    pub is_hardware: bool,
    pub serial: Option<u32>,
    pub timebase: f64,
}

unsafe impl Send for Decoder {}

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
    pub fn new(stream: &Stream, options: &[(&str, &str)]) -> Result<Decoder, Error> {
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

            let mut is_hardware = false;
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
                    is_hardware = true;
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
                timebase: stream.timebase,
                is_hardware,
            })
        }
    }

    pub fn receive_frame(&mut self, frame: &mut Frame) -> DecoderResult {
        frame.receive_frame_from_decoder(self.serial, self.timebase, self.context)
    }

    pub fn send_packet(&mut self, packet: &Packet) -> Result<(), Error> {
        unsafe {
            if let Some(serial) = self.serial {
                if serial != packet.serial {
                    avcodec_flush_buffers(self.context);
                    self.serial = Some(packet.serial);
                }
            } else {
                avcodec_flush_buffers(self.context);
                self.serial = Some(packet.serial);
            }

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

pub struct AudioConverter {
    context: *mut SwrContext,
    channels: u32,
    sample_rate: u32,
    sample_bytes: u32,
    is_planar: bool,
    sample_format: AVSampleFormat,
    channel_layout: AVChannelLayout,
}

unsafe impl Send for AudioConverter {}
unsafe impl Sync for AudioConverter {}

impl AudioConverter {
    pub fn new(channels: u32, sample_rate: u32, sample_format: AVSampleFormat) -> AudioConverter {
        unsafe {
            let mut channel_layout: AVChannelLayout = AVChannelLayout {
                order: AVChannelOrder::FF_CHANNEL_ORDER_NB,
                nb_channels: 0,
                u: AVChannelLayout__bindgen_ty_1 {
                    mask: 0
                },
                opaque: null_mut()
            };
            av_channel_layout_default(&mut channel_layout, channels as i32);
            AudioConverter {
                context: swr_alloc(),
                channels,
                sample_rate,
                sample_format,
                sample_bytes: av_get_bytes_per_sample(sample_format) as u32,
                is_planar: av_sample_fmt_is_planar(sample_format) == 1,
                channel_layout
            }
        }
    }

    pub fn convert_frame(&mut self, frame: &Frame, dest: &mut AudioFrame) -> Result<(), Error> {
        unsafe {
            let samples_num = frame.num_samples() as i32;
            if self.context.is_null() {
                return Err(Error::from_code(-1));
            }

            let result = swr_alloc_set_opts2(
                &mut self.context,
                &self.channel_layout,
                self.sample_format,
                self.sample_rate as i32,
                &frame.channel_layout(),
                frame.sample_format().unwrap(),
                frame.sample_rate() as i32,
                0,
                null_mut()
            );

            if result < 0 {
                return Err(Error::from_code(result));
            }

            let result = swr_init(self.context);
            if result < 0 {
                return Err(Error::from_code(result));
            }

            let samples_out = swr_get_out_samples(self.context, samples_num);
            let planes = if self.is_planar { self.channels } else { 1 };
            let buffer_size = if self.is_planar {
                samples_out as u32 * self.sample_bytes
            } else {
                samples_out as u32 * self.sample_bytes * self.channels
            };
            dest.ensure_allocated(buffer_size as usize, planes as usize);
            let src = (*frame.pointer).data.as_ptr() as *const *const u8;
            let dest_ptr = dest.planes.as_ptr();
            let result = swr_convert(self.context, dest_ptr, samples_out, src, samples_num);
            if result < 0 {
                return Err(Error::from_code(result))
            }
            dest.channels = self.channels as usize;
            dest.sample_rate = self.sample_rate;
            dest.num_samples = if self.is_planar { self.channels as usize } else { 1 } * result as usize;
            dest.pts = frame.pts;
            dest.duration = frame.duration;
            dest.serial = frame.serial;

            Ok(())
        }
    }
}

impl Drop for AudioConverter {
    fn drop(&mut self) {
        unsafe {
            if !self.context.is_null() {
                swr_free(&mut self.context);
            }
        }
    }
}