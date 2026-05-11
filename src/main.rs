use std::collections::HashMap;
use wgpu::{Color, Face, FragmentState, FrontFace, MultisampleState, Operations, PipelineCompilationOptions, PipelineLayoutDescriptor, PolygonMode, PrimitiveState, PrimitiveTopology, RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderSource, StoreOp, TextureView, VertexState};
use wgpu::LoadOp::Clear;
use wgpu::wgt::CommandEncoderDescriptor;
use crate::ffmpeg::decode::{Decoder, DecoderResult};
use crate::ffmpeg::frame::Frame;
use crate::ffmpeg::input::{Input, StreamType};
use crate::window::app::{AppContext, Scene, State};

pub mod window;
pub mod ffmpeg;

struct PlayerScene {
    render_pipeline: Option<wgpu::RenderPipeline>,
}

impl PlayerScene {
    fn new() -> Self {
        Self {
            render_pipeline: None,
        }
    }
}

impl Scene for PlayerScene {
    fn init(&mut self, state: &mut State, app: &mut AppContext) {
        let shader = state.device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Simple rendering shader"),
            source: ShaderSource::Wgsl(include_str!("res/render.wgsl").into()),
        });
        let pipeline_layout = state.device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Simple rendering pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0
        });
        let render_pipeline = state.device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Simple rendering pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[]
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: Some(Face::Back),
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[]
            }),
            multiview_mask: None,
            cache: None
        });
    }

    fn render(&mut self, state: &mut State, app: &mut AppContext, view: &TextureView) {
        let mut encoder = state.device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });
        {
            let pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: Clear(Color { r: 1.0, g: 0.5, b: 0.3, a: 1.0 }),
                        store: StoreOp::Store
                    }
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None
            });
        }

        state.queue.submit(std::iter::once(encoder.finish()));
    }
}

fn main() {
    let mut input = Input::open("test.mp4", vec![]).unwrap();
    let stream = input.streams.iter().find(|stream| stream.stream_type == StreamType::Video).unwrap().clone();
    let mut decoder = Decoder::new(stream.clone(), vec![]).unwrap();
    let mut frame = Frame::new();
    while let Ok(packet) = input.read_packet() {
        if packet.stream_index() != stream.index {
            continue;
        }
        loop {
            let result = decoder.receive_frame(&mut frame);
            match result {
                DecoderResult::NeedsInput => {
                    decoder.send_packet(packet.clone()).unwrap();
                    break
                }
                DecoderResult::Error(error) => {
                    panic!("Error decoding: {:?}", error);
                }
                DecoderResult::FrameReceived => {
                    println!("Frame received: {} s", frame.pts.unwrap());
                }
            }
        }
        frame.unref();
    }

    let mut app = window::app::App::new();
    app.app_context.set_scene(Box::new(PlayerScene::new()));
    app.run().unwrap();
}
