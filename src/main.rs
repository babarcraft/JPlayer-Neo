mod ffmpeg;
pub mod gs;
pub mod player;

use crate::gs::gl::clear_current_buffer_color;
use crate::gs::nvg::NvgInstance;
use crate::gs::window::{Window, WindowHandler};
use glfw::{PWindow, WindowEvent};
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::rc::Rc;
use crate::ffmpeg::packet::ByteBuffer;
use crate::player::cache::CacheFile;
use crate::player::ui::UIManager;

struct App {
    ui: UIManager,
    handle: Rc<RefCell<PWindow>>
}

impl App {
    pub fn new(window: &Window) -> Self {
        let mut nvg_context = NvgInstance::new();

        nvg_context.load_font("default", "src/res/def.ttf");

        let mut ui = UIManager::new(&nvg_context, window);
        ui.load_script(Path::new("out.lua")).unwrap();

        Self {
            ui, 
            handle: window.handle()
        }
    }
}

impl WindowHandler for App {
    fn initialize(&mut self, window: &Window) {}

    fn render(&mut self, dt: f32, window: &Window) {
        clear_current_buffer_color();

        let handle = self.handle.borrow();
        let (w, h) = handle.get_framebuffer_size();
        self.ui.render(w as f32, h as f32).unwrap();
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