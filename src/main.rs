mod ffmpeg;
pub mod gs;
pub mod player;

use crate::ffmpeg::frame::Frame;
use crate::gs::gl::clear_current_buffer_color;
use crate::gs::nvg::{Color, NvgContext, Shape, Text};
use crate::gs::window::{Window, WindowHandler};
use crate::player::decoder::DecodeWorker;
use crate::player::input::InputWorker;
use crate::player::player::VideoPlayer;
use crate::player::surface::VideoSurface;
use glfw::{Action, Key, MouseButton};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;
use crate::gs::nvg;

struct App {
    frame_sw: Frame,
    input_worker: InputWorker,
    decode_worker: DecodeWorker,
    video_surface: Rc<RefCell<VideoSurface>>,
    video_player: Option<VideoPlayer>,
    nvg_image: Option<i32>,
    test_img: Option<i32>,
    text: Text,
    nvg_context: NvgContext,
    current_index: usize,
    current_page: usize,
    begin: Instant,
    last_decode_passes: usize,
    last_input_passes: usize,
    timeline_bounds: ((f32, f32), (f32, f32)),
}

impl App {
    pub fn new() -> Self {
        let surface = VideoSurface::new();
        let video_surface = Rc::new(RefCell::new(surface));
        let mut nvg_context = NvgContext::new();

        nvg_context.load_font("default", "src/res/def.ttf");
        nvg_context.set_font("default", 64.0);

        let text = nvg_context.text(100.0, 100.0, "Input: ");
        Self {
            frame_sw: Frame::new(),
            video_surface,
            nvg_image: None,
            video_player: None,
            input_worker: InputWorker::new(),
            decode_worker: DecodeWorker::new(),
            nvg_context,
            text,
            begin: Instant::now(),
            last_decode_passes: 0,
            last_input_passes: 0,
            current_index: 0,
            current_page: 15,
            test_img: None,
            timeline_bounds: ((0.0, 0.0), (0.0, 0.0)),
        }
    }
}

impl WindowHandler for App {
    fn initialize(&mut self, window: &mut Window) {}

    fn render(&mut self, dt: f32, window: &mut Window) {
        clear_current_buffer_color();

        if let Some(playback) = &mut self.video_player {
            playback.render_update();
        }

        let (w, h) = window.get_size();
        self.nvg_context.frame((w, h), |context| {
            context.update_text(&mut self.text);
            
            let text = &self.text;
            context.begin_path();
            context.fill_color(nvg::Color::gray(1.0, 1.0));
            context.draw_text(text);
            context.fill();
            context.begin_path();
            context.fill_shape_color(Shape::Rect(0.0, 0.0, 100.0, 100.0), Color::gray(0.5, 1.0));
            let (x, y, w, h) = text.bounds();
            context.fill_shape_color(Shape::Rect(x, y, w, h), Color::rgb(0.0, 1.0, 1.0).alpha(0.5));
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

    fn handle_char(&mut self, c: char, window: &Window) {
        self.text.push_back(c)
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
    let mut window = Window::new("Test", 1000, 1000).unwrap();

    let mut app = App::new();

    window.run(&mut app);
}