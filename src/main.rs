mod ffmpeg;
pub mod gs;
pub mod player;

use std::time::{Instant, SystemTime};
use glfw::{Action, Context, Key, WindowHint};
use crate::gs::buffer::{ElementBuffer, LayoutElement, LayoutElementStep, LayoutElementType, VertexArray, VertexBuffer};
use crate::gs::gl::check_errors;
use crate::gs::shader::{Shader, UniformValue};
use crate::gs::texture::{InternalFormat, Texture};

struct App {
    vbo: VertexBuffer,
    ebo: ElementBuffer,
    vao: VertexArray,
    texture: Texture,
    compute_shader: Shader,
    shader: Shader,
}




fn main() {
    let mut glfw = glfw::init(glfw::fail_on_errors).expect("Failed to init GLFW");

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
        -0.5, -0.5,    0.0, 0.0,
        0.5, -0.5,    0.0, 1.0,
        0.5, 0.5,    1.0, 1.0,
        -0.5, 0.5,    1.0, 0.0,
    ]);
    let mut ebo = ElementBuffer::new();
    ebo.upload_u16(&[
        0, 1, 2,
        0, 3, 2
    ]);
    let mut vao = VertexArray::new();
    vao.attach_vertex_buffer(&vbo, &[
        LayoutElement { layout_element: LayoutElementType::Float, count: 2, step: LayoutElementStep::Vertex },
        LayoutElement { layout_element: LayoutElementType::Float, count: 2, step: LayoutElementStep::Vertex },
    ]);
    vao.attach_element_buffer(&ebo);

    let mut texture = Texture::new();
    texture.upload(None, None, 1000, 1000, InternalFormat::Rgba(8));
    texture.bind(None);
    texture.set_parameters(gl::LINEAR, gl::LINEAR, gl::CLAMP_TO_EDGE, gl::CLAMP_TO_EDGE);
    texture.unbind();

    let comp_shader = Shader::compile_compute(include_str!("res/test_comp.glsl")).unwrap();
    comp_shader.bind();
    let coef_uniform = comp_shader.get_uniform_location("coef").unwrap();
    comp_shader.unbind();

    let shader = Shader::compile(include_str!("res/test.vert"), include_str!("res/test.frag")).unwrap();
    let begin = Instant::now();

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

        comp_shader.bind();
        comp_shader.set_uniform(coef_uniform, &UniformValue::Float(begin.elapsed().as_secs_f32().cos()));
        texture.bind_image(0);
        comp_shader.dispatch_compute((texture.width.unwrap() + 15) / 16, (texture.height.unwrap() + 15) / 16, 1);
        comp_shader.image_access_barrier();
        comp_shader.unbind();

        shader.bind();
        texture.bind(Some(0));
        vao.draw_indexed(gl::TRIANGLES, 6, gl::UNSIGNED_SHORT);
        shader.unbind();
        check_errors("Render", true);

        window.swap_buffers();
    }
}