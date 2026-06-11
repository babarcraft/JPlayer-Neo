mod ffmpeg;
pub mod gs;
pub mod player;

use crate::gs::gl::clear_current_buffer_color;
use crate::gs::nvg::NvgInstance;
use crate::gs::window::{Window, WindowHandler};
use glfw::WindowEvent;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use crate::player::ui::UIManager;

struct App {
    ui: UIManager
}

impl App {
    pub fn new(window: &Window) -> Self {
        let mut nvg_context = NvgInstance::new();

        nvg_context.load_font("default", "src/res/def.ttf");

        let mut ui = UIManager::new(&nvg_context, window);
        ui.load_script(Path::new("out.lua")).unwrap();

        Self {
            ui
        }
    }
}

impl WindowHandler for App {
    fn initialize(&mut self, window: &mut Window) {}

    fn render(&mut self, dt: f32, window: &mut Window) {
        clear_current_buffer_color();

        let (w, h) = window.get_framebuffer_size();
        self.ui.render(w, h).unwrap();
    }

    fn handle_event(&mut self, event: WindowEvent, window: &Window) {
        self.ui.handle_event(event).unwrap();
    }
}

fn main() {
    let mut window = Window::new("Test", 1000, 1000).unwrap();
    let mut app = App::new(&window);

    window.run(&mut app);
}