mod ffmpeg;
pub mod gs;

use glfw::{Action, Context, Key, WindowHint};

fn main() {
    let mut glfw = glfw::init(glfw::fail_on_errors)
        .expect("Failed to init GLFW");

    // Request OpenGL 4.5 Core
    glfw.window_hint(WindowHint::ContextVersion(4, 5));
    glfw.window_hint(WindowHint::OpenGlProfile(
        glfw::OpenGlProfileHint::Core,
    ));

    let (mut window, events) = glfw.create_window(
            800,
            600,
            "OpenGL 4.5",
            glfw::WindowMode::Windowed,
        ).expect("Failed to create window");

    window.make_current();
    window.set_key_polling(true);

    gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);

    unsafe {
        gl::ClearColor(0.2, 0.3, 0.3, 1.0);
    }

    while !window.should_close() {
        glfw.poll_events();

        for (_, event) in glfw::flush_messages(&events) {
            match event {
                glfw::WindowEvent::Key(Key::Escape, _, Action::Press, _) => {
                    window.set_should_close(true)
                }
                glfw::WindowEvent::Char(character) => {
                    println!("{}", character);
                }
                _ => {}
            }
        }

        unsafe {
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }

        window.swap_buffers();
    }
}