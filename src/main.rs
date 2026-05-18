mod ffmpeg;
pub mod gs;
pub mod player;

use std::sync::mpsc::Receiver;
use std::thread::{JoinHandle, Thread};
use std::time::{Instant, SystemTime};
use gl::types::{GLint, GLuint};
use glfw::{Action, Context, Key, WindowHint};
use crate::ffmpeg::decode::{Decoder, DecoderResult};
use crate::ffmpeg::frame::Frame;
use crate::ffmpeg::input::{Input, Stream, StreamType};
use crate::gs::buffer::{ElementBuffer, LayoutElement, LayoutElementStep, LayoutElementType, PixelBuffer, VertexArray, VertexBuffer};
use crate::gs::gl::check_errors;
use crate::gs::shader::{Shader, UniformValue};
use crate::gs::texture::{InternalFormat, Texture};
use crate::gs::window::{Window, WindowHandler};
use std::sync::{Arc, Mutex, MutexGuard, Condvar};

struct App {
    vbo: VertexBuffer,
    ebo: ElementBuffer,
    vao: VertexArray,
    texture: Texture,

    compute_shader: Shader,
    comp_y: GLint,
    comp_uv: GLint,

    shader: Shader,

    thread: JoinHandle<()>,
    receiver: Receiver<Frame>,

    stream: Stream,

    begin: Instant,

    frame_sw: Frame,
    last_frame: Option<Frame>,

    y_plane: Option<(PixelBuffer, Texture)>,
    uv_plane: Option<(PixelBuffer, Texture)>,
}

impl App {
    pub fn new() -> Self {
        let mut vbo = VertexBuffer::new();

        vbo.upload_f32(&[
            // pos        // uv
            -1.0, -1.0,  0.0, 1.0,
            1.0, -1.0,  1.0, 1.0,
            1.0,  1.0,  1.0, 0.0,
            -1.0,  1.0,  0.0, 0.0,
        ]);

        let mut ebo = ElementBuffer::new();

        ebo.upload_u16(&[
            0, 1, 2,
            0, 3, 2,
        ]);

        let mut vao = VertexArray::new();

        vao.attach_vertex_buffer(
            &vbo,
            &[
                LayoutElement {
                    layout_element: LayoutElementType::Float,
                    count: 2,
                    step: LayoutElementStep::Vertex,
                },
                LayoutElement {
                    layout_element: LayoutElementType::Float,
                    count: 2,
                    step: LayoutElementStep::Vertex,
                },
            ],
        );

        vao.attach_element_buffer(&ebo);

        let texture = Texture::new();

        let mut input = Input::open("test_o.mp4", vec![]).unwrap();

        let stream = input
            .streams
            .iter()
            .find(|stream| stream.stream_type == StreamType::Video)
            .unwrap()
            .clone();

        let (sender, receiver) = std::sync::mpsc::sync_channel(15);

        let thread = std::thread::spawn(move || {
            let stream = input
                .streams
                .iter()
                .find(|stream| stream.stream_type == StreamType::Video)
                .unwrap();

            let index = stream.index;

            let mut decoder = Decoder::new(stream.clone(), vec![]).unwrap();
            let mut frame = Frame::new();

            while let Some(packet) = input.read_packet().ok() {
                if packet.stream_index() != index {
                    continue;
                }

                loop {
                    match decoder.receive_frame(&mut frame) {
                        DecoderResult::FrameReceived => {
                            sender.send(frame.clone()).unwrap();
                            frame.unref();
                        }

                        DecoderResult::Error(error) => {
                            println!("DECODER ERROR: {:?}", error);
                        }

                        DecoderResult::NeedsInput => {
                            decoder.send_packet(&packet).unwrap();
                            break;
                        }
                    }
                }
            }
        });

        let comp_shader =
            Shader::compile_compute(include_str!("res/test_comp.glsl")).unwrap();

        comp_shader.bind();

        let comp_y = comp_shader.get_uniform_location("texY").unwrap();
        let comp_uv = comp_shader.get_uniform_location("texUV").unwrap();

        comp_shader.unbind();

        let shader = Shader::compile(
            include_str!("res/test.vert"),
            include_str!("res/test.frag"),
        )
            .unwrap();

        Self {
            vbo,
            ebo,
            vao,
            texture,

            compute_shader: comp_shader,
            comp_y,
            comp_uv,

            shader,

            thread,
            receiver,

            stream,

            begin: Instant::now(),

            frame_sw: Frame::new(),
            last_frame: None,

            y_plane: None,
            uv_plane: None,
        }
    }
}

impl WindowHandler for App {
    fn initialize(&mut self, window: &mut Window) {}

    fn render(&mut self, dt: f32, window: &mut Window) {
        if self.last_frame.is_none() {
            self.last_frame = self.receiver.try_recv().ok();
        }

        let mut frame = None;

        if let Some(last_frame) = self.last_frame.take() {
            let pts = last_frame.pts.unwrap();
            let duration = last_frame.duration.unwrap();

            let range = pts..pts + duration;

            let clock = self.begin.elapsed().as_secs_f64();

            if range.contains(&clock) {
                self.frame_sw.unref();

                last_frame
                    .transfer_hw_data_to(&mut self.frame_sw, &self.stream)
                    .unwrap();

                frame = Some(&self.frame_sw);
            } else if clock < pts {
                self.last_frame = Some(last_frame);
            }
        }

        if let Some(frame) = frame {
            if !self.texture.has_space(
                frame.width() as u32,
                frame.height() as u32,
                InternalFormat::Rgba(8),
            ) {
                self.texture.bind(Some(0));

                self.texture.upload(
                    None,
                    None,
                    frame.width() as u32,
                    frame.height() as u32,
                    InternalFormat::Rgba(8),
                );

                self.texture.set_parameters(
                    gl::LINEAR,
                    gl::LINEAR,
                    gl::CLAMP_TO_EDGE,
                    gl::CLAMP_TO_EDGE,
                );

                self.texture.unbind();
            }

            if let None = self.y_plane {
                let y_buffer = PixelBuffer::allocate_persistent(
                    frame.height() * frame.plane_stride(0),
                    None,
                )
                    .unwrap();

                let mut y_texture = Texture::new();

                y_texture.bind(Some(0));

                y_texture.upload(
                    None,
                    None,
                    frame.width() as u32,
                    frame.height() as u32,
                    InternalFormat::R(8),
                );

                y_texture.set_parameters(
                    gl::LINEAR,
                    gl::LINEAR,
                    gl::CLAMP_TO_EDGE,
                    gl::CLAMP_TO_EDGE,
                );

                y_texture.unbind();

                self.y_plane = Some((y_buffer, y_texture));
            }

            if let None = self.uv_plane {
                let uv_buffer = PixelBuffer::allocate_persistent(
                    (frame.height() / 2) * frame.plane_stride(1),
                    None,
                )
                    .unwrap();

                let mut uv_texture = Texture::new();

                uv_texture.bind(Some(1));

                uv_texture.upload(
                    None,
                    None,
                    (frame.width() / 2) as u32,
                    (frame.height() / 2) as u32,
                    InternalFormat::Rg(8),
                );

                uv_texture.set_parameters(
                    gl::LINEAR,
                    gl::LINEAR,
                    gl::CLAMP_TO_EDGE,
                    gl::CLAMP_TO_EDGE,
                );

                uv_texture.unbind();

                self.uv_plane = Some((uv_buffer, uv_texture));
            }

            self.compute_shader.bind();

            if let Some((pbo, texture)) = &mut self.y_plane {
                pbo.mapped()
                    .unwrap()
                    .copy_from_slice(frame.plane(0, 2));

                unsafe {
                    gl::MemoryBarrier(gl::CLIENT_MAPPED_BUFFER_BARRIER_BIT);
                }

                pbo.bind();

                texture.bind(Some(0));

                texture.upload_partial(
                    None,
                    Some(frame.plane_stride(0)),
                    0,
                    0,
                    frame.width() as u32,
                    frame.height() as u32,
                );

                pbo.unbind();
            }

            if let Some((pbo, texture)) = &mut self.uv_plane {
                pbo.mapped()
                    .unwrap()
                    .copy_from_slice(frame.plane(1, 2));

                unsafe {
                    gl::MemoryBarrier(gl::CLIENT_MAPPED_BUFFER_BARRIER_BIT);
                }

                check_errors("Upload uv", true);

                pbo.bind();

                texture.bind(Some(1));

                texture.upload_partial(
                    None,
                    Some(frame.plane_stride(1)),
                    0,
                    0,
                    (frame.width() / 2) as u32,
                    (frame.height() / 2) as u32,
                );

                pbo.unbind();
            }

            self.texture.bind_image(2);

            self.compute_shader.dispatch_compute(
                (frame.width() + 15) as u32 / 16,
                (frame.height() + 15) as u32 / 16,
                1,
            );

            self.compute_shader.image_access_barrier();
            self.compute_shader.unbind();
        }

        self.shader.bind();

        self.texture.bind(Some(0));

        self.vao
            .draw_indexed(gl::TRIANGLES, 6, gl::UNSIGNED_SHORT);

        self.texture.unbind();

        self.shader.unbind();
    }
}

fn main() {
    let mut window = Window::new("Test", 1000, 1000).unwrap();

    let mut app = App::new();

    window.run(&mut app);
}