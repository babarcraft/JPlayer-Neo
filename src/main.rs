mod ffmpeg;
pub mod gs;
pub mod player;

use crate::ffmpeg::input::Input;
use crate::gs::gl::clear_current_buffer_color;
use crate::gs::nvg::{Color, NvgContext, TextHorizontalAlignment, TextVerticalAlignment};
use crate::gs::window::{Window, WindowHandler};
use crate::player::decoder::DecodeWorker;
use crate::player::input::InputWorker;
use crate::player::player::VideoPlayer;
use crate::player::surface::VideoSurface;
use crate::player::ui::{Component, ComponentBody, ComponentId, ComponentManager, UIManager};
use glfw::{Action, Key, WindowEvent};
use mlua::{AnyUserData, Lua, Value};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;

struct App {
    input_worker: InputWorker,
    decode_worker: DecodeWorker,
    nvg_context: NvgContext,
    ui: UIManager
}

impl App {
    pub fn new() -> Self {
        let mut nvg_context = NvgContext::new();

        nvg_context.load_font("default", "src/res/def.ttf");

        let mut input_worker = InputWorker::new();
        let mut decode_worker = DecodeWorker::new();


        let mut ui = UIManager::new();
        ui.exec(std::fs::read_to_string("ui.lua").unwrap());

        Self {
            input_worker,
            decode_worker,
            nvg_context,
            ui
        }
    }
}

impl WindowHandler for App {
    fn initialize(&mut self, window: &mut Window) {}

    fn render(&mut self, dt: f32, window: &mut Window) {
        clear_current_buffer_color();

        let (w, h) = window.get_size();

        self.nvg_context.frame((w, h), |context| {
            self.ui.update(context).unwrap();
            self.ui.run_updates().unwrap();
            self.ui.render(context);
        });
    }

    fn handle_event(&mut self, event: WindowEvent, window: &Window) {
        match event.clone() {
            WindowEvent::FramebufferSize(..) => {
                self.ui.dirty();
            }
            _ => {}
        }
    }
}

fn main() {
    let mut window = Window::new("Test", 1000, 1000).unwrap();

    let mut app = App::new();

    window.run(&mut app);
}