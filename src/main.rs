mod ffmpeg;
pub mod gs;
pub mod player;

use std::cell::RefCell;
use crate::ffmpeg::frame::Frame;
use crate::ffmpeg::input::{Input, Stream, StreamType};
use crate::gs::nvg::NvgContext;
use crate::gs::texture::InternalFormat;
use crate::gs::window::{Window, WindowHandler};
use crate::player::decoder::{AudioRingClock, DecodeWorker, DecodeWorkerMessage};
use crate::player::input::{InputCommand, InputWorker};
use crate::player::surface::{FrameQueue, VideoSurface};
use glfw::{Action, Context, Key, MouseButton};
use std::ops::Div;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::sync::atomic::Ordering;
use std::time::Instant;
use bytemuck::PodCastError::SizeMismatch;
use cpal::traits::{DeviceTrait, HostTrait};
use nanovg_sys::nvgFontFace;
use crate::gs::gl::clear_current_buffer_color;
use player::player::VideoPlayback;
use crate::player::audio::AudioDevice;
use crate::player::clock::Clock;
use crate::player::player::VideoPlayer;

struct App {
    frame_sw: Frame,
    input_worker: InputWorker,
    decode_worker: DecodeWorker,
    video_surface: Rc<RefCell<VideoSurface>>,
    video_player: Option<VideoPlayer>,
    nvg_image: Option<i32>,
    nvg_context: NvgContext,
    begin: Instant,
    last_decode_passes: usize,
    last_input_passes: usize,
    timeline_bounds: ((f32, f32), (f32, f32)),
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
            nvg_image: None,
            video_player: None,
            input_worker: InputWorker::new(),
            decode_worker: DecodeWorker::new(),
            nvg_context,
            begin: Instant::now(),
            last_decode_passes: 0,
            last_input_passes: 0,
            timeline_bounds: ((0.0, 0.0), (0.0, 0.0)),
        }
    }
}

impl WindowHandler for App {
    fn initialize(&mut self, window: &mut Window) {}

    fn render(&mut self, dt: f32, window: &mut Window) {
        clear_current_buffer_color();

        if let Some(playback) = self.video_player.as_mut() {
            playback.render_update();
        }
        let mut video_surface = self.video_surface.borrow_mut();
        if let Some((width, height)) = video_surface.size_update.take() {
            self.nvg_image = Some(self.nvg_context.create_texture_image(&video_surface.output_texture));
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
                let h = 0.07;
                let size = context.relative(0.8, h);
                let offset = context.relative(0.1, 0.01);
                self.timeline_bounds = (offset, (offset.0 + size.0, offset.1 + size.1));
                context.rect(offset, size);
                context.fill_color((1.0, 1.0, 1.0, 1.0));
                context.fill();
                let progress = self.video_player.as_ref()
                    .map(|player| player.current_pts() / player.estimated_duration)
                    .unwrap_or(0.0) as f32;
                context.begin_path();
                context.rect(offset, context.relative(0.8 * progress, h));
                context.fill_color((0.3, 0.3, 0.3, 1.0));
                context.fill();
            }
        });
    }

    fn handle_key(&mut self, key: Key, action: Action, window: &Window) {
        match key {
            Key::Escape => {
                if action == Action::Press {
                    self.video_player.take();
                }
            }
            Key::Enter => {
                if action == Action::Press {
                    let url = window.get_clipboard().unwrap_or("".to_string());
                    let input = Input::open(url.as_str(), &[
                    ]);
                    match input {
                        Ok(input) => {
                            self.video_player = Some(VideoPlayer::new(input, Some(self.video_surface.clone()), &mut self.decode_worker, &mut self.input_worker));
                        }
                        Err(e) => {
                            eprintln!("{:?}", e);
                        }
                    }
                }
            }
            Key::Right => {
                if action == Action::Press {
                    if let Some(video_player) = self.video_player.as_mut() {
                        video_player.seek(video_player.current_pts() + 5.0)
                    }
                }
            }
            Key::Left => {
                if action == Action::Press {
                    if let Some(video_player) = self.video_player.as_mut() {
                        video_player.seek(video_player.current_pts() - 5.0)
                    }
                }
            }

            Key::Space => {
                if action == Action::Press {
                    if let Some(video_player) = self.video_player.as_mut() {
                        if video_player.is_playing() {
                            video_player.pause();
                        } else {
                            video_player.play();
                        }
                    }
                }
            }

            Key::F => {
                if action == Action::Press {
                }
            }
            _ => {}
        }
    }

    fn handle_mouse_button(&mut self, button: MouseButton, action: Action, window: &Window) {
        if action != Action::Press {
            return;
        }
        let player = match &mut self.video_player {
            Some(player) => player,
            None => return,
        };
        let (x, y) = window.mouse_position;
        let ((tx0, ty0), (tx1, ty1)) = self.timeline_bounds;
        if x >= tx0 && x <= tx1 && y >= ty0 && y <= ty1 {
            let percent = (x - tx0) / (tx1 - tx0);
            player.seek(player.estimated_duration * percent as f64)
        }
    }

    fn handle_mouse_position(&mut self, x: f32, y: f32, window: &Window) {
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