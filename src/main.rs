mod ffmpeg;
pub mod gs;
pub mod player;

use glfw::{Action, Context, Key, WindowHint};
use crate::gs::buffer::{ElementBuffer, LayoutElement, LayoutElementStep, LayoutElementType, VertexArray, VertexBuffer};
use crate::gs::shader::Shader;

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    colors: [f32; 3],
}

fn main() {
    let mut glfw = glfw::init(glfw::fail_on_errors)
        .expect("Failed to init GLFW");

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

    let mut vbo = VertexBuffer::new();
    vbo.upload_f32(&[
        -0.5, -0.5,    1.0, 1.0, 1.0,
        -0.5, 0.5,    1.0, 1.0, 1.0,
        0.5, 0.5,    1.0, 1.0, 1.0,
    ]);
    let mut ebo = ElementBuffer::new();
    ebo.upload_u16(&[
        0, 1, 2
    ]);
    let mut vao = VertexArray::new();
    vao.attach_vertex_buffer(&vbo, &[
        LayoutElement { layout_element: LayoutElementType::Float, count: 2, step: LayoutElementStep::Vertex },
        LayoutElement { layout_element: LayoutElementType::Float, count: 3, step: LayoutElementStep::Vertex },
    ]);
    vao.attach_element_buffer(&ebo);

    let shader = Shader::compile(include_str!("res/test.vert"), include_str!("res/test.frag")).unwrap();

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

        shader.bind();
        vao.draw_indexed(gl::TRIANGLES, 3, gl::UNSIGNED_SHORT);
        shader.unbind();

        window.swap_buffers();
    }
}