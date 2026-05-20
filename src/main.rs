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

struct App {
    begin: Instant,
    frame_sw: Frame,
    input_worker: InputWorker,
    decode_worker: DecodeWorker,
    sender: Sender<DecodeWorkerMessage>,
    frame_queue: Arc<RwLock<FrameQueue>>,
    video_surface: VideoSurface,
    nvg_context: NvgContext,
    nvg_image: Option<i32>,
}

impl App {
    pub fn new() -> Self {
        let input = Input::open("test_o.mp4", vec![]).unwrap();

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
        let mut decode_streams = decode_worker.begin_decode(&queues, input, &mut input_worker);

        let (sender, queue) = decode_streams[video_stream.index as usize].take().unwrap();

        let surface = VideoSurface::new();
        let nvg_context = NvgContext::new();

        Self {
            begin: Instant::now(),
            frame_sw: Frame::new(),
            video_surface: surface,
            sender,
            frame_queue: queue,
            input_worker,
            decode_worker,
            nvg_context,
            nvg_image: None,
        }
    }
}

impl WindowHandler for App {
    fn initialize(&mut self, window: &mut Window) {}

    fn render(&mut self, dt: f32, window: &mut Window) {
        {
            let queue = self.frame_queue.read().unwrap();
            if let Some(frame) = queue.peek_read() {
                let pts = frame.pts.unwrap();
                let duration = frame.duration.unwrap();

                let range = pts..pts + duration;

                let clock = self.begin.elapsed().as_secs_f64();

                if range.contains(&clock) {
                    self.video_surface.upload(frame, &[
                        InternalFormat::R(8),
                        InternalFormat::Rg(8),
                    ], Some(2));
                    self.video_surface.convert_output();

                    self.nvg_image.get_or_insert_with(|| {
                        self.nvg_context.create_texture_image(&self.video_surface.output_texture)
                    });

                    let should_notify = !queue.has_space();
                    drop(queue);
                    let mut queue = self.frame_queue.write().unwrap();
                    queue.pop();
                    if should_notify {
                        self.sender.send(DecodeWorkerMessage::Wakeup).unwrap();
                    }
                } else if clock < pts {
                } else {
                    drop(queue);
                    let mut queue = self.frame_queue.write().unwrap();
                    queue.pop();
                    self.sender.send(DecodeWorkerMessage::Wakeup).unwrap();
                }
            }
        }

        let secs = (self.begin.elapsed().as_secs() as usize).max(1);

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