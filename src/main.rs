mod ffmpeg;
pub mod gs;
pub mod player;

use std::ops::Div;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::{JoinHandle, Thread};
use std::time::{Instant, SystemTime};
use gl::types::{GLint, GLuint};
use glfw::{Action, Context, Key, WindowHint};
use crate::ffmpeg::decode::{Decoder, DecoderResult};
use crate::ffmpeg::frame::Frame;
use crate::ffmpeg::input::{Input, Stream, StreamType};
use crate::gs::buffer::{ElementBuffer, LayoutElement, LayoutElementStep, LayoutElementType, PixelBuffer, VertexArray, VertexBuffer};
use crate::gs::gl::{check_errors, mapped_buffer_barrier};
use crate::gs::shader::{Shader, UniformValue};
use crate::gs::texture::{InternalFormat, Texture};
use crate::gs::window::{Window, WindowHandler};
use std::sync::{Arc, Mutex, MutexGuard, Condvar, RwLock};
use ffmpeg_sys_next::av_opt_query_ranges;
use nanovg_sys::NVGpaint;
use crate::gs::nvg::NvgContext;
use crate::player::surface::{FrameQueue, VideoSurface};

struct App {
    vbo: VertexBuffer,
    ebo: ElementBuffer,
    vao: VertexArray,
    texture: Texture,
    shader: Shader,
    thread: JoinHandle<()>,
    sender: Sender<bool>,
    frame_queue: Arc<RwLock<FrameQueue>>,
    stream: Stream,
    begin: Instant,
    frame_sw: Frame,
    last_frame: Option<Frame>,
    video_surface: VideoSurface,
    nvg_context: NvgContext,
    nvg_image: Option<i32>,
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

        let (sender, receiver) = std::sync::mpsc::channel();
        let frame_queue = Arc::new(RwLock::new(FrameQueue::new(15)));

        let frame_queue_clone = frame_queue.clone();
        let thread = std::thread::spawn(move || {
            let stream = input
                .streams
                .iter()
                .find(|stream| stream.stream_type == StreamType::Video)
                .unwrap();

            let index = stream.index;

            let mut decoder = Decoder::new(&stream, vec![]).unwrap();
            let mut frame = Frame::new();

            while let Some(packet) = input.read_packet().ok() {
                if packet.stream_index() != index {
                    continue;
                }

                loop {
                    match decoder.receive_frame(&mut frame) {
                        DecoderResult::FrameReceived => {
                            let mut wait = false;
                            loop {
                                if wait {
                                    receiver.recv().unwrap();
                                }
                                let mut lock = frame_queue_clone.write().unwrap();
                                if let Some(copy_frame) = lock.peek_write() {
                                    copy_frame.unref();
                                    frame.transfer_hw_data_to(copy_frame).unwrap();
                                    lock.push();
                                    break;
                                } else {
                                    wait = true;
                                    continue;
                                }
                            }
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

        let shader = Shader::compile(
            include_str!("res/test.vert"),
            include_str!("res/test.frag"),
        ).unwrap();

        let surface = VideoSurface::new();
        let nvg_context = NvgContext::new();

        Self {
            vbo,
            ebo,
            vao,
            texture,
            shader,
            thread,
            frame_queue,
            sender,
            stream,
            begin: Instant::now(),
            frame_sw: Frame::new(),
            video_surface: surface,
            last_frame: None,
            nvg_context,
            nvg_image: None,
        }
    }
}

impl WindowHandler for App {
    fn initialize(&mut self, window: &mut Window) {}

    fn render(&mut self, dt: f32, window: &mut Window) {
        {
            let queue = self.frame_queue.read().unwrap();
            if let Some(frame) = queue.peek_read() {
                let pts = frame.pts.unwrap();
                let duration = frame.duration.unwrap();

                let range = pts..pts + duration;

                let clock = self.begin.elapsed().as_secs_f64();

                if range.contains(&clock) {
                    self.video_surface.upload(frame, &[
                        InternalFormat::R(8),
                        InternalFormat::Rg(8),
                    ], Some(2));
                    self.video_surface.convert_output();

                    self.nvg_image.get_or_insert_with(|| {
                        self.nvg_context.create_texture_image(&self.video_surface.output_texture)
                    });

                    let should_notify = !queue.has_space();
                    drop(queue);
                    let mut queue = self.frame_queue.write().unwrap();
                    queue.pop();
                    if should_notify {
                        self.sender.send(false).unwrap();
                    }
                } else if clock < pts {
                } else {
                    drop(queue);
                    let mut queue = self.frame_queue.write().unwrap();
                    queue.pop();
                    self.sender.send(false).unwrap();
                }
            }
        }

        let (w, h) = window.get_size();
        self.nvg_context.frame((w, h), |context| {
            if let Some(image) = self.nvg_image {
                let vw = self.video_surface.output_texture.width.unwrap() as f32;
                let vh = self.video_surface.output_texture.height.unwrap() as f32;
                let s = (w / vw).min(h / vh);

                let pw = s * vw;
                let ph = s * vh;
                let ox = (w - pw).div(2.0).max(0.0);
                let oy = (h - ph).div(2.0).max(0.0);

                let paint = context.image_paint(image, (ox, oy), (pw, ph));
                context.set_fill_paint(paint);
                context.begin_path();
                context.rect((ox, oy), (pw, ph));
                context.fill();
            }
        });
    }
}

fn main() {
    let mut window = Window::new("Test", 1000, 1000).unwrap();

    let mut app = App::new();

    window.run(&mut app);
}