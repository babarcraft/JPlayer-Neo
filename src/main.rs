mod ffmpeg;
pub mod gs;
pub mod player;

use crate::ffmpeg::frame::Frame;
use crate::ffmpeg::input::{Input, Stream, StreamType};
use crate::gs::nvg::NvgContext;
use crate::gs::texture::InternalFormat;
use crate::gs::window::{Window, WindowHandler};
use crate::player::decoder::{DecodeWorker, DecodeWorkerMessage};
use crate::player::input::InputWorker;
use crate::player::surface::{FrameQueue, VideoSurface};
use glfw::Context;
use std::ops::Div;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use cpal::traits::{DeviceTrait, HostTrait};
use nanovg_sys::nvgFontFace;
use crate::gs::gl::clear_current_buffer_color;
use crate::player::audio::AudioDevice;
use crate::player::clock::Clock;

struct App {
    frame_sw: Frame,
    input_worker: InputWorker,
    decode_worker: DecodeWorker,
    device: AudioDevice,
    sender: Sender<DecodeWorkerMessage>,
    frame_queue: Arc<RwLock<FrameQueue>>,
    video_surface: VideoSurface,
    nvg_context: NvgContext,
    nvg_image: Option<i32>,
    num_frames: usize,
    num_fq_fail: usize,
    num_cl_fail: usize,
    num_dropped: usize,
    num_begin: Instant,
    begin: Instant,
    last_pts: Option<f64>,
    last_duration: f64,
    frame_timer: Option<f64>,
}

impl App {
    pub(crate) fn present_frame(&mut self, p0: &Frame) {
        todo!()
    }
}

impl App {
    pub fn new() -> Self {
        let input = Input::open("test.mp4", vec![]).unwrap();

        let audio_stream = input
            .streams
            .iter()
            .find(|stream| stream.stream_type == StreamType::Audio)
            .unwrap()
            .clone();
        let video_stream = input
            .streams
            .iter()
            .find(|stream| stream.stream_type == StreamType::Video)
            .unwrap()
            .clone();
        let mut queues: Vec<Option<&Stream>> = (0..input.streams.len()).map(|_| None).collect();
        queues[video_stream.index as usize] = Some(&video_stream);
        queues[audio_stream.index as usize] = Some(&audio_stream);
        let mut decode_worker = DecodeWorker::new();
        let mut input_worker = InputWorker::new();
        let mut decode_streams = decode_worker.begin_decode(&queues, Some((44100, 1)), input, &mut input_worker);

        let mut device = {
            let (sender, queue) = decode_streams[audio_stream.index as usize].take().unwrap();
            AudioDevice::default_device(queue.unwrap_audio(), sender).unwrap()
        };

        device.play();

        let (sender, queue) = decode_streams[video_stream.index as usize].take().unwrap();

        let surface = VideoSurface::new();
        let mut nvg_context = NvgContext::new();

        nvg_context.load_font("default", "src/res/def.ttf");
        nvg_context.set_font("default", 32.0);

        Self {
            frame_sw: Frame::new(),
            video_surface: surface,
            sender,
            frame_queue: queue.unwrap_video(),
            device,
            input_worker,
            decode_worker,
            nvg_context,
            nvg_image: None,
            num_frames: 0,
            num_cl_fail: 0,
            num_dropped: 0,
            num_fq_fail: 0,
            last_pts: None,
            last_duration: 0.0,
            frame_timer: None,
            begin: Instant::now(),
            num_begin: Instant::now(),
        }
    }
}

impl WindowHandler for App {
    fn initialize(&mut self, window: &mut Window) {}

    fn render(&mut self, dt: f32, window: &mut Window) {
        clear_current_buffer_color();

        let current_time = self.begin.elapsed().as_secs_f64();

        let audio_clock = self.device.ring_buffer.try_read().ok()
            .and_then(|ring| ring.pts_interpolated())
            .unwrap_or(0.0) as f64;

        let mut should_pop = false;
        let mut queue_was_full = false;

        let mut av = 0.0;
        let mut frame_num = 0;

        if let Some(queue) = self.frame_queue.try_read().ok() {
            queue_was_full = !queue.has_space();
            frame_num = queue.queued();

            if let Some(frame) = queue.peek_read() {
                let current_pts = frame.pts.unwrap_or(0.0) as f64;
                av = audio_clock - current_pts;

                if self.frame_timer.is_none() || self.last_pts.is_none() {
                    self.frame_timer = Some(current_time);
                    self.last_pts = Some(current_pts);
                    self.last_duration = frame.duration.unwrap_or(0.04) as f64;

                    self.video_surface.upload(frame, &[
                        InternalFormat::R(8),
                        InternalFormat::Rg(8),
                    ], Some(2));
                    self.video_surface.convert_output();
                    self.nvg_image.get_or_insert_with(|| {
                        self.nvg_context.create_texture_image(&self.video_surface.output_texture)
                    });
                    self.num_frames += 1;
                    should_pop = true;
                } else {
                    let last_pts = self.last_pts.unwrap();

                    let mut duration = current_pts - last_pts;
                    if duration <= 0.0 || duration > 1.0 {
                        duration = self.last_duration; // fallback to previous valid entry
                    }

                    let diff = current_pts - audio_clock;
                    let sync_threshold = 0.04_f64.max(0.1_f64.min(duration));

                    let mut delay = duration;
                    if diff.abs() < 3600.0 {
                        if diff <= -sync_threshold {
                            delay = 0.0_f64.max(duration + diff);
                        } else if diff >= sync_threshold {
                            delay = duration + diff;
                        }
                    }

                    let target_time = self.frame_timer.unwrap() + delay;

                    if current_time < target_time {
                    } else {
                        self.frame_timer = Some(target_time);

                        if current_time - target_time > 0.1 {
                            self.frame_timer = Some(current_time);
                        }

                        if diff < -0.1 {
                            self.num_dropped += 1;
                            should_pop = true;
                        } else {
                            self.video_surface.upload(frame, &[
                                InternalFormat::R(8),
                                InternalFormat::Rg(8),
                            ], Some(2));
                            self.video_surface.convert_output();
                            self.nvg_image.get_or_insert_with(|| {
                                self.nvg_context.create_texture_image(&self.video_surface.output_texture)
                            });
                            self.num_frames += 1;
                            self.last_pts = Some(current_pts);
                            self.last_duration = duration;
                            should_pop = true;
                        }
                    }
                }
            }
        } else {
            self.num_fq_fail += 1;
        }

        // 4. If the logic determined we need to pop, acquire the write lock safely outside our read block
        if should_pop {
            if let Some(mut write_queue) = self.frame_queue.try_write().ok() {
                write_queue.pop();
                if queue_was_full {
                    let _ = self.sender.send(DecodeWorkerMessage::Wakeup);
                }
            }
        }
        let (w, h) = window.get_size();
        self.nvg_context.frame((w, h), |context| {
            if let Some(image) = self.nvg_image {
                let vw = self.video_surface.output_texture.width.unwrap() as f32;
                let vh = self.video_surface.output_texture.height.unwrap() as f32;
                let s = (w / vw).min(h / vh);

                let pw = s * vw;
                let ph = s * vh;
                let ox = (w - pw).div(2.0).max(0.0);
                let oy = (h - ph).div(2.0).max(0.0);

                let paint = context.image_paint(image, (ox, oy), (pw, ph));
                context.set_fill_paint(paint);
                context.begin_path();
                context.rect((ox, oy), (pw, ph));
                context.fill();

                context.begin_path();
                let elapsed = self.num_begin.elapsed().as_secs().max(1) as usize;
                context.text((10.0, 10.0), format!("FPS: {}", self.num_frames / elapsed).as_str());
                context.text((10.0, 30.0), format!("FQF: {}", self.num_fq_fail / elapsed).as_str());
                context.text((10.0, 50.0), format!("CLF: {}", self.num_cl_fail / elapsed).as_str());
                context.text((10.0, 70.0), format!("DPS: {}", self.num_dropped / elapsed).as_str());
                context.text((10.0, 90.0), format!("A-V diff: {:.2}", av).as_str());
                context.text((10.0, 110.0), format!("Frame queue size: {}", frame_num).as_str());
            }
        });
    }
}

fn main() {
    let host = cpal::default_host();
    let default_output = host.default_output_device().unwrap();
    for config in default_output.supported_output_configs().unwrap() {
        println!("Supported output config: {:?}", config);
    }

    let mut window = Window::new("Test", 1000, 1000).unwrap();

    let mut app = App::new();

    window.run(&mut app);
}