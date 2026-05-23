use std::collections::HashMap;
use std::mem::replace;
use std::sync::mpsc::Receiver;
use std::time::Instant;
use glfw::{Action, Context, Glfw, GlfwReceiver, Key, MouseButton, PWindow, WindowEvent, WindowHint};
use crate::gs::gl::{check_errors, ErrorType};
use crate::gs::shader::UniformValue;

pub trait WindowHandler {
    fn initialize(&mut self, window: &mut Window);
    fn render(&mut self, dt: f32, window: &mut Window);
    fn handle_key(&mut self, key: Key, action: Action, window: &Window);
    fn handle_mouse_button(&mut self, button: MouseButton, action: Action, window: &Window);
    fn handle_mouse_position(&mut self, x: f32, y: f32, window: &Window);
}

pub struct Window {
    glfw: Glfw,
    window: PWindow,
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
        window.set_framebuffer_size_polling(true);
        window.set_size_polling(true);

        gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);
        
        let window = Window {
            glfw,
            window,
            mouse_position: (0.0, 0.0),
            events,
        };
        Some(window)
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        self.window.set_size(width as i32, height as i32);
    }
    
    pub fn run(&mut self, handler: &mut dyn WindowHandler) {
        handler.initialize(self);

        let mut manual_change = false;
        let mut last = Some(Instant::now());
        while !self.window.should_close() {
            self.glfw.poll_events();

            for (_, event) in glfw::flush_messages(&self.events) {
                match event {
                    glfw::WindowEvent::Key(key, _scancode, action, _mods) => {
                        handler.handle_key(key, action, self);
                    }
                    glfw::WindowEvent::Char(character) => {
                        println!("{}", character);
                    }
                    glfw::WindowEvent::FramebufferSize(width, height) => {
                        unsafe {
                            gl::Viewport(0, 0, width, height);
                        }
                    }
                    glfw::WindowEvent::Size(w, h) => {
                    }
                    glfw::WindowEvent::MouseButton(button, action, _) => {
                        handler.handle_mouse_button(button, action, self);
                    }
                    glfw::WindowEvent::CursorPos(x, y) => {
                        self.mouse_position = (x as f32, self.window.get_size().1 as f32 - y as f32);
                        handler.handle_mouse_position(self.mouse_position.0, self.mouse_position.1, self);
                    }
                    _ => {}
                }
            }
            
            if let Some(time) = last.take() {
                handler.render(time.elapsed().as_secs_f32(), self);
                last = Some(Instant::now());
            } else {
                last = Some(Instant::now());
            }

            check_errors("Render", true);

            self.window.swap_buffers();
        }
    }

    pub fn get_clipboard(&self) -> Option<String> {
        self.window.get_clipboard_string()
    }

    pub fn is_fullscreen(&self) -> bool {
        todo!()
    }

    pub fn get_size(&self) -> (f32, f32) {
        let (w, h) = self.window.get_size();
        (w as f32, h as f32)
    }
}