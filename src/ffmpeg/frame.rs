use std::ffi::c_void;
use std::io::Chain;
use std::ptr::null_mut;
use ffmpeg_sys_next::{av_frame_alloc, av_frame_clone, av_frame_copy_props, av_frame_free, av_frame_move_ref, av_frame_unref, av_free, av_hwframe_transfer_data, av_malloc, avcodec_receive_frame, AVChannelLayout, AVCodecContext, AVColorRange, AVColorSpace, AVFrame, AVPixelFormat, AVSampleFormat, AVERROR, EAGAIN, EOF};
use crate::ffmpeg::decode::DecoderResult;
use crate::ffmpeg::error::Error;
use crate::ffmpeg::frame::SampleType::Float;
use crate::ffmpeg::input::Stream;
use crate::ffmpeg::utils::*;


#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct ColorInfo {
    color_space: [[f32; 3]; 3],
    color_offset: [f32; 3],
    others: [f32; 4],
}

#[derive(Debug)]
pub struct Frame {
    pub(crate) pointer: *mut AVFrame,
    pub serial: Option<u32>,
    pub pts: Option<f64>,
    pub duration: Option<f64>,
    pub timebase: Option<f64>,
}

unsafe impl Send for Frame {}
unsafe impl Sync for Frame {}

impl Frame {
    pub fn new() -> Self {
        unsafe {
            let frame = av_frame_alloc();
            Self {
                pointer: frame,
                serial: None,
                pts: None,
                duration: None,
                timebase: None,
            }
        }
    }

    pub(super) fn receive_frame_from_decoder(&mut self, serial: Option<u32>, timebase: f64, context: *mut AVCodecContext) -> DecoderResult {
        unsafe {
            let result = avcodec_receive_frame(context, self.pointer);
            if result < 0 {
                if result == AVERROR(EAGAIN) || result == AVERROR(EOF) {
                    DecoderResult::NeedsInput
                } else {
                    DecoderResult::Error(Error::from_code(result))
                }
            } else {
                self.timebase = Some(timebase);
                self.update_data(serial);

                DecoderResult::FrameReceived
            }
        }
    }

    fn update_data(&mut self, serial: Option<u32>) {
        unsafe {
            let timebase = match self.timebase {
                Some(timebase) => timebase,
                None => return,
            };

            self.serial = serial;
            self.pts = Some((*self.pointer).pts as f64 * timebase);
            self.duration = Some((*self.pointer).duration as f64 * timebase);
        }
    }

    pub fn transfer_hw_data_to(&self, other: &mut Frame) -> Result<(), Error> {
        unsafe {
            let result = av_hwframe_transfer_data(other.pointer, self.pointer, 0);
            if result < 0 {
                return Err(Error::from_code(result))
            }
            let result = av_frame_copy_props(other.pointer, self.pointer);
            if result < 0 {
                return Err(Error::from_code(result))
            }
            other.timebase = self.timebase;
            other.update_data(other.serial);
            Ok(())
        }
    }

    pub fn sample_format(&self) -> Option<AVSampleFormat> {
        unsafe {
            std::mem::transmute((*self.pointer).format)
        }
    }

    pub fn sample_rate(&self) -> u32 {
        unsafe {
            (*self.pointer).sample_rate as u32
        }
    }

    pub fn channel_layout(&self) -> AVChannelLayout {
        unsafe {
            (*self.pointer).ch_layout
        }
    }

    pub fn num_samples(&self) -> usize {
        unsafe {
            (*self.pointer).nb_samples as usize
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

    pub fn color_range_vec(&self) -> [f32; 3] {
        unsafe {
            match (*self.pointer).color_range {
                AVColorRange::AVCOL_RANGE_MPEG => {
                    [-16.0 / 255.0, -0.5, -0.5, ]
                }
                AVColorRange::AVCOL_RANGE_JPEG => {
                    [0.0, -0.5, -0.5, ]
                }
                AVColorRange::AVCOL_RANGE_UNSPECIFIED => {
                    [-16.0 / 255.0, -0.5, -0.5, ]
                }
                AVColorRange::AVCOL_RANGE_NB => {
                    [-16.0 / 255.0, -0.5, -0.5, ]
                }
            }
        }
    }

    pub fn color_space_matrix(&self) -> [[f32; 3]; 3] {
        unsafe {
            match (*self.pointer).colorspace {
                AVColorSpace::AVCOL_SPC_BT709 => bt709(),

                AVColorSpace::AVCOL_SPC_BT470BG => bt601(),
                AVColorSpace::AVCOL_SPC_SMPTE170M => bt601(),
                AVColorSpace::AVCOL_SPC_SMPTE240M => bt601(),
                AVColorSpace::AVCOL_SPC_FCC => bt601(),

                AVColorSpace::AVCOL_SPC_BT2020_NCL => bt2020(),
                AVColorSpace::AVCOL_SPC_BT2020_CL => bt2020(),

                AVColorSpace::AVCOL_SPC_RGB => rgb(),

                AVColorSpace::AVCOL_SPC_UNSPECIFIED => fallback(),

                _ => fallback(),
            }
        }
    }

    pub fn plane(&self, num: usize, chroma: Option<usize>) -> &[u8] {
        unsafe {
            let line_size = (*self.pointer).linesize[num] as usize;
            let data = (*self.pointer).data[num];
            if data.is_null() {
                panic!("Invalid access of frame data! Plane {num} not found!")
            }
            let height = ((*self.pointer).height as usize) / chroma.clone().take_if(|_| num > 0).unwrap_or(1);
            std::slice::from_raw_parts(data, line_size * height)
        }
    }
    
    pub fn plane_stride(&self, num: usize) -> usize {
        unsafe {
            (*self.pointer).linesize[num] as usize
        }
    }

    pub fn move_to(&self, other: &mut Frame) {
        other.unref();
        unsafe {
            av_frame_move_ref(other.pointer, self.pointer);
        }
        other.timebase = self.timebase;
        other.update_data(other.serial);
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
                timebase: self.timebase,
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

pub enum SampleType {
    Float
}

pub struct AudioFrame {
    pub planes: [*mut u8; 8],
    pub plane_sizes: [Option<usize>; 8],
    pub num_planes: usize,
    pub channels: usize,
    pub sample_rate: u32,
    pub sample_type: SampleType,
    pub num_samples: usize,
    pub pts: Option<f64>,
    pub duration: Option<f64>,
    pub serial: Option<u32>
}

unsafe impl Send for AudioFrame {}
unsafe impl Sync for AudioFrame {}

impl AudioFrame {
    pub fn new() -> Self {
        Self {
            planes: [const { null_mut() }; 8],
            plane_sizes: [const { None }; 8],
            num_planes: 0,
            sample_type: Float,
            channels: 0,
            sample_rate: 0,
            num_samples: 0,
            pts: None,
            duration: None,
            serial: None
        }
    }

    pub fn plane<T>(&self, num: usize) -> &[T] {
        unsafe {
            std::slice::from_raw_parts(self.planes[num] as *const T, self.plane_sizes[num].unwrap() / size_of::<T>())
        }
    }

    pub fn ensure_allocated(&mut self, plane_size: usize, planes: usize) {
        for index in 0..planes {
            if let Some(size) = &mut self.plane_sizes[index] {
                if *size != plane_size {
                    self.planes[index] = unsafe {
                        av_malloc(plane_size) as *mut u8
                    };
                    *size = plane_size;
                }
            } else {
                self.planes[index] = unsafe {
                    av_malloc(plane_size) as *mut u8
                };
                self.plane_sizes[index] = Some(plane_size);
            }
        }
        for i in planes..planes.max(self.num_planes) {
            let plane_ptr = &mut self.planes[i];
            if !plane_ptr.is_null() {
                unsafe {
                    av_free(*plane_ptr as *mut c_void);
                }
                *plane_ptr = null_mut();
            }
        }
        self.num_planes = planes;
    }
}

impl Drop for AudioFrame {
    fn drop(&mut self) {
        for plane in &self.planes[0..self.num_planes] {
            unsafe {
                av_free(*plane as *mut c_void);
            }
        }
    }
}