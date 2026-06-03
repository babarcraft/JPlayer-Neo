mod ffmpeg;
pub mod gs;
pub mod player;

use std::process::Child;
use crate::ffmpeg::frame::Frame;
use crate::gs::gl::clear_current_buffer_color;
use crate::gs::nvg::{Color, NvgContext, Point, Shape, Text, TextHorizontalAlignment, TextVerticalAlignment};
use crate::gs::window::{Window, WindowHandler};
use crate::player::decoder::DecodeWorker;
use crate::player::input::InputWorker;
use crate::player::player::VideoPlayer;
use crate::player::surface::VideoSurface;
use glfw::{Action, Key, MouseButton, WindowEvent};
use std::cell::RefCell;
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::Instant;
use ffmpeg_sys_next::{labs, perror};
use crate::ffmpeg::input::Input;
use crate::gs::nvg;
use crate::player::ui::{Component, ComponentBody, ComponentId, ComponentManager};

struct App {
    input_worker: InputWorker,
    decode_worker: DecodeWorker,
    nvg_context: NvgContext,
    component_manager: ComponentManager,

    controls_timeline: ComponentId,
    controls_time_passed: ComponentId,
    controls_time_rem: ComponentId,
    controls_root: ComponentId,
    player: VideoPlayer,
    video_surface: ComponentId,
    stats: ComponentId
}

impl App {
    pub fn new() -> Self {
        let mut nvg_context = NvgContext::new();

        nvg_context.load_font("default", "src/res/def.ttf");

        let mut component_manager = ComponentManager::new();

        let surface = Rc::new(RefCell::new(VideoSurface::new()));

        let video_surface = component_manager.push(Component::video_surface(surface.clone()));
        let controls_timeline = component_manager.push(Component::slider(Color::gray(0.4, 1.0), Color::gray(0.7, 1.0)));
        let controls_time_passed = component_manager.push(Component::label(&mut nvg_context, Color::gray(1.0, 1.0), (TextHorizontalAlignment::Left, TextVerticalAlignment::Center)));
        let controls_time_rem = component_manager.push(Component::label(&mut nvg_context, Color::gray(1.0, 1.0), (TextHorizontalAlignment::Right, TextVerticalAlignment::Center)));
        let stats = component_manager.push(Component::label(&mut nvg_context, Color::gray(1.0, 1.0), (TextHorizontalAlignment::Center, TextVerticalAlignment::Center)));

        let hg = component_manager.push(Component::hgroup(vec![
            (1.0, Some(controls_time_passed)),
            (3.0, None),
            (1.0, Some(controls_time_rem)),
        ]));
        let vg = component_manager.push(Component::vgroup(vec![
            (5.0, Some(stats)),
            (2.0, Some(hg)),
            (0.75, None),
            (3.0, Some(controls_timeline)),
            (5.0, None),
        ]).with_padding(15.0));
        let back_rect = component_manager.push(Component::rect(Color::rgb(1.0, 0.5, 0.0)));
        let controls_root = component_manager.push(Component::root(vec![back_rect, vg]).with_padding(25.0).with_preferred_size(None, Some(200.0)));
        let root = component_manager.push(Component::root(vec![video_surface, controls_root]));

        component_manager.set_root(root);

        let mut input_worker = InputWorker::new();
        let mut decode_worker = DecodeWorker::new();
        let input = Input::open("tt.webm", &[]).unwrap();
        let mut player = VideoPlayer::new(input, Some(&mut *surface.borrow_mut()), &mut decode_worker, &mut input_worker).unwrap();
        player.play();

        Self {
            input_worker,
            decode_worker,
            nvg_context,
            component_manager,
            player,

            video_surface,
            controls_timeline,
            controls_time_passed,
            controls_time_rem,
            controls_root,
            stats
        }
    }
}

impl WindowHandler for App {
    fn initialize(&mut self, window: &mut Window) {}

    fn render(&mut self, dt: f32, window: &mut Window) {
        clear_current_buffer_color();

        let (w, h) = window.get_size();

        if let Some(ComponentBody::Label { text, .. }) = self.component_manager.get_mut_body(self.controls_time_passed) {
            text.clear();
            let mut seconds = self.player.master_clock.pts();
            let hours = (seconds / 3600.0) as u32;
            seconds -= hours as f64 * 3600.0;
            let minutes = (seconds / 60.0) as u32;
            seconds -= minutes as f64 * 60.0;
            let seconds = seconds as u32;
            text.push_str(&format!("{:02}:{:02}:{:02}", hours, minutes, seconds));
        }
        if let Some(ComponentBody::Label { text, .. }) = self.component_manager.get_mut_body(self.controls_time_rem) {
            text.clear();
            let mut seconds = self.player.estimated_duration - self.player.master_clock.pts();
            let hours = (seconds / 3600.0) as u32;
            seconds -= hours as f64 * 3600.0;
            let minutes = (seconds / 60.0) as u32;
            seconds -= minutes as f64 * 60.0;
            let seconds = seconds as u32;
            text.push_str(&format!("-{:02}:{:02}:{:02}", hours, minutes, seconds));
        }
        if let Some(ComponentBody::Label { text, .. }) = self.component_manager.get_mut_body(self.stats) {
            text.clear();
            text.push_str(&format!("Input passes: {:012} Decode passes: {:012}", self.input_worker.passes.load(Ordering::Relaxed), self.decode_worker.passes.load(Ordering::Relaxed)));
        }
        if let Some(ComponentBody::Slider { percent, target, foreground, background }) =
            self.component_manager.get_mut_body(self.controls_timeline) {
            *percent = (self.player.master_clock.pts() / self.player.estimated_duration) as f32;
            if let Some(target) = target.take() {
                self.player.seek(target as f64 * self.player.estimated_duration);
            }
        }

        self.nvg_context.frame((w, h), |context| {
            self.component_manager.render_root(context);
        });
    }

    fn handle_event(&mut self, event: WindowEvent, window: &Window) {
        self.component_manager.handle_event(event, window);
    }
}

fn main() {
    let mut window = Window::new("Test", 1000, 1000).unwrap();

    let mut app = App::new();

    window.run(&mut app);
}