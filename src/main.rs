use glfw::{Action, Context, Key, WindowHint};
use glow::HasContext;

fn main() {
    let mut glfw = glfw::init(glfw::fail_on_errors)
        .expect("Failed to init GLFW");

    // Request OpenGL 4.5 Core
    glfw.window_hint(WindowHint::ContextVersion(4, 5));
    glfw.window_hint(WindowHint::OpenGlProfile(
        glfw::OpenGlProfileHint::Core,
    ));

    let (mut window, events) = glfw
        .create_window(
            800,
            600,
            "OpenGL 4.5",
            glfw::WindowMode::Windowed,
        )
        .expect("Failed to create window");

    window.make_current();
    window.set_key_polling(true);

    let gl = unsafe {
        glow::Context::from_loader_function(|s| {
            window.get_proc_address(s) as *const _
        })
    };

    unsafe {
        gl.clear_color(0.1, 0.2, 0.3, 1.0);
    }

    while !window.should_close() {
        glfw.poll_events();

        for (_, event) in glfw::flush_messages(&events) {
            match event {
                glfw::WindowEvent::Key(Key::Escape, _, Action::Press, _) => {
                    window.set_should_close(true)
                }
                _ => {}
            }
        }

        unsafe {
            gl.clear(glow::COLOR_BUFFER_BIT);
        }

        window.swap_buffers();
    }
}