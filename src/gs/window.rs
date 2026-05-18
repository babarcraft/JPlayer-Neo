use std::mem::replace;
use std::sync::mpsc::Receiver;
use std::time::Instant;
use glfw::{Action, Context, Glfw, GlfwReceiver, Key, PWindow, WindowEvent, WindowHint};
use crate::gs::gl::{check_errors, ErrorType};
use crate::gs::shader::UniformValue;

pub trait WindowHandler {
    fn initialize(&mut self, window: &mut Window);
    fn render(&mut self, dt: f32, window: &mut Window);
}

pub struct Window {
    glfw: Glfw,
    window: PWindow,
    events: GlfwReceiver<(f64, WindowEvent)>,
}

impl Window {
    pub fn new(title: &str, width: u32, height: u32) -> Option<Window> {
        let mut glfw = glfw::init(glfw::fail_on_errors).expect("Failed to init GLFW");

        glfw.window_hint(WindowHint::ContextVersion(4, 5));
        glfw.window_hint(WindowHint::OpenGlProfile(glfw::OpenGlProfileHint::Core, ));

        let (mut window, events) = 
            glfw.create_window(width, height, title, glfw::WindowMode::Windowed,)?;

        window.make_current();
        window.set_key_polling(true);

        gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);
        
        let window = Window {
            glfw,
            window,
            events,
        };
        Some(window)
    }
    
    pub fn run(&mut self, handler: &mut dyn WindowHandler) {
        handler.initialize(self);

        let mut last = Some(Instant::now());
        while !self.window.should_close() {
            self.glfw.poll_events();

            for (_, event) in glfw::flush_messages(&self.events) {
                match event {
                    glfw::WindowEvent::Key(Key::Escape, _, Action::Press, _) => {
                        self.window.set_should_close(true)
                    }
                    glfw::WindowEvent::Char(character) => {
                        println!("{}", character);
                    }
                    _ => {}
                }
            }
            
            if let Some(time) = last.take() {
                handler.render(time.elapsed().as_secs_f32(), self)
            } else {
                last = Some(Instant::now());
            }

            check_errors("Render", true);

            self.window.swap_buffers();
        }
    }
}