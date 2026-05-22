mod ffmpeg;
pub mod gs;
pub mod player;

use std::cell::RefCell;
use crate::ffmpeg::frame::Frame;
use crate::ffmpeg::input::{Input, Stream, StreamType};
use crate::gs::nvg::NvgContext;
use crate::gs::texture::InternalFormat;
use crate::gs::window::{Window, WindowHandler};
use crate::player::decoder::{DecodeWorker, DecodeWorkerMessage};
use crate::player::input::{InputCommand, InputWorker};
use crate::player::surface::{FrameQueue, VideoSurface};
use glfw::{Action, Context, Key};
use std::ops::Div;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::sync::atomic::Ordering;
use std::time::Instant;
use cpal::traits::{DeviceTrait, HostTrait};
use nanovg_sys::nvgFontFace;
use crate::gs::gl::clear_current_buffer_color;
use player::player::VideoPlayback;
use crate::player::audio::AudioDevice;
use crate::player::clock::Clock;

struct App {
    frame_sw: Frame,
    input_worker: InputWorker,
    decode_worker: DecodeWorker,
    video_surface: Rc<RefCell<VideoSurface>>,
    video_playback: Option<VideoPlayback>,
    audio_device: Option<AudioDevice>,
    command_sender: Option<Sender<InputCommand>>,
    nvg_image: Option<i32>,
    nvg_context: NvgContext,
    begin: Instant,
    last_decode_passes: usize,
    last_input_passes: usize,
}

impl App {
    pub(crate) fn present_frame(&mut self, p0: &Frame) {
        todo!()
    }
}

impl App {
    pub fn new() -> Self {

        let surface = VideoSurface::new();
        let video_surface = Rc::new(RefCell::new(surface));
        let mut nvg_context = NvgContext::new();

        nvg_context.load_font("default", "src/res/def.ttf");
        nvg_context.set_font("default", 32.0);

        Self {
            frame_sw: Frame::new(),
            video_surface,
            video_playback: None,
            audio_device: None,
            nvg_image: None,
            command_sender: None,
            input_worker: InputWorker::new(),
            decode_worker: DecodeWorker::new(),
            nvg_context,
            begin: Instant::now(),
            last_decode_passes: 0,
            last_input_passes: 0,
        }
    }
}

impl WindowHandler for App {
    fn initialize(&mut self, window: &mut Window) {}

    fn render(&mut self, dt: f32, window: &mut Window) {
        clear_current_buffer_color();

        if let Some(playback) = self.video_playback.as_mut() {
            playback.update();
        }
        let mut video_surface = self.video_surface.borrow_mut();
        if let Some((width, height)) = video_surface.size_update.take() {
            self.nvg_image = Some(self.nvg_context.create_texture_image(&video_surface.output_texture));
        }

        if let Some(serial) = self.audio_device.as_ref()
            .and_then(|device| device.ring_buffer.read().ok()
                .and_then(|ring| ring.serial())) {
        }

        let (w, h) = window.get_size();
        self.nvg_context.frame((w, h), |context| {
            if let Some(image) = self.nvg_image {
                let vw = video_surface.output_texture.width.unwrap() as f32;
                let vh = video_surface.output_texture.height.unwrap() as f32;
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
                let current_decode = self.decode_worker.passes.load(Ordering::Relaxed);
                let current_input = self.input_worker.passes.load(Ordering::Relaxed);
                context.text((100.0, 100.0), format!("Rate of Input pass {:.2}", (current_decode as f64/ 1000f64) / self.begin.elapsed().as_secs_f64()).as_str());
                context.text((100.0, 120.0), format!("Rate of Input pass {:.2}", (current_input as f64/ 1000f64) / self.begin.elapsed().as_secs_f64()).as_str());
                let playback_data = self.video_playback.as_ref().map(|play| (play.seek.unwrap_or(-1.0), play.last_pts.unwrap_or(-1.0))).unwrap_or((-1.0, -1.0));
                context.text((100.0, 140.0), format!("Amount buffered {:.2}", self.audio_device.as_ref().map(|device| device.ring_buffer.read().unwrap().buffered()).unwrap_or(0.0)).as_str());

                let mut offset = 160.0;
                for (name, num) in self.decode_worker.wake_ups.read().unwrap().iter() {
                    context.text((100.0, offset), format!("Rate of {} {:.2}", *name, (*num as f64) / self.begin.elapsed().as_secs_f64()).as_str());
                    offset += 20.0;
                }

                self.last_decode_passes = current_decode;
                self.last_input_passes = current_input;
            }
        });
    }

    fn handle_key(&mut self, key: Key, action: Action, window: &Window) {
        match key {
            Key::Escape => {
                if action == Action::Press {
                    self.video_playback.take();
                    self.audio_device.take();
                    self.command_sender.take();
                }
            }
            Key::Enter => {
                if action == Action::Press {
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
                    let (mut decode_streams, command_sender) = self.decode_worker.begin_decode(&queues, Some((44100, 1)), input, &mut self.input_worker);

                    let mut device = {
                        let (sender, queue) = decode_streams[audio_stream.index as usize].take().unwrap();
                        AudioDevice::default_device(queue.unwrap_audio(), sender).unwrap()
                    };

                    let (sender, queue) = decode_streams[video_stream.index as usize].take().unwrap();
                    let playback = VideoPlayback::new(queue.unwrap_video(), sender, self.video_surface.clone(), device.ring_buffer.clone());
                    self.video_playback = Some(playback);
                    self.audio_device = Some(device);
                    self.command_sender = Some(command_sender);
                }
            }
            Key::Right => {
                if action == Action::Press {
                    if let Some(sender) = &self.command_sender {
                        if let Some(device) = &self.audio_device {
                            let mut ring = device.ring_buffer.write().unwrap();
                            let target = ring.pts_interpolated().unwrap_or(0.0) + 5.0;
                            sender.send(InputCommand::Seek(0.0, target, None)).unwrap();
                        }
                    }
                }
            }
            Key::Left => {
                if action == Action::Press {
                    if let Some(sender) = &self.command_sender {
                        if let Some(device) = &self.audio_device {
                            let mut ring = device.ring_buffer.write().unwrap();
                            let target = ring.pts_interpolated().unwrap_or(0.0) - 5.0;
                            sender.send(InputCommand::Seek(0.0, target, None)).unwrap();
                        }
                    }
                }
            }

            Key::Space => {
                if action == Action::Press {
                    if let Some(device) = &mut self.audio_device {
                        if let Some(playback) = self.video_playback.as_mut() {
                            if device.is_playing() {
                                device.pause();
                                playback.playing = false;
                            } else {
                                device.play();
                                playback.playing = true;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
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