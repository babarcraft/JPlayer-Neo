use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::replace;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Instant;
use glfw::{Action, Context, FlushedMessages, Glfw, GlfwReceiver, Key, MouseButton, PWindow, WindowEvent, WindowHint};
use crate::gs::gl::{check_errors, ErrorType};
use crate::gs::shader::UniformValue;

pub trait WindowHandler {
    fn initialize(&mut self, window: &Window);
    fn render(&mut self, dt: f32, window: &Window);
    fn handle_event(&mut self, event: WindowEvent, window: &Window);
}

pub struct Window {
    glfw: Glfw,
    handle: Rc<RefCell<PWindow>>,
    events: GlfwReceiver<(f64, WindowEvent)>,
    pub mouse_position: (f32, f32),
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
        window.set_mouse_button_polling(true);
        window.set_cursor_pos_polling(true);
        window.set_char_polling(true);
        window.set_framebuffer_size_polling(true);
        window.set_size_polling(true);

        gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);
        
        let window = Window {
            glfw,
            handle: Rc::new(RefCell::new(window)),
            mouse_position: (0.0, 0.0),
            events,
        };
        Some(window)
    }

    pub fn run(&mut self, handler: &mut dyn WindowHandler) {
        handler.initialize(self);

        let mut manual_change = false;
        let mut last = Some(Instant::now());
        loop {
            {
                let mut handle = self.handle.borrow_mut();
                if handle.should_close() { break; }
            }
            self.glfw.poll_events();

            for (time, event) in glfw::flush_messages(&self.events) {
                match event {
                    glfw::WindowEvent::FramebufferSize(width, height) => {
                        unsafe {
                            gl::Viewport(0, 0, width, height);
                        }
                    }
                    _ => {}
                }
                handler.handle_event(event, self);
            }
            
            if let Some(time) = last.take() {
                handler.render(time.elapsed().as_secs_f32(), self);
                last = Some(Instant::now());
            } else {
                last = Some(Instant::now());
            }

            check_errors("Render", true);

            {
                let mut handle = self.handle.borrow_mut();
                handle.swap_buffers();
            }
        }
    }
    
    pub fn events(&self) -> FlushedMessages<'_, (f64, WindowEvent)> {
        glfw::flush_messages(&self.events)
    }
    
    pub fn handle(&self) -> Rc<RefCell<PWindow>> {
        self.handle.clone()
    }
}