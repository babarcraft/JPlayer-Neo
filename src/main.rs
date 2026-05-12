use std::collections::HashMap;
use std::ops::{Add, Range};
use std::time::{Duration, Instant, SystemTime};
use ffmpeg_sys_next::{avformat_receive_command_reply, daylight};
use wgpu::{BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType, BlendState, BufferAddress, BufferUsages, Color, ColorTargetState, ColorWrites, Extent3d, Face, FragmentState, FrontFace, IndexFormat, MultisampleState, Operations, Origin3d, PipelineCompilationOptions, PipelineLayoutDescriptor, PolygonMode, PrimitiveState, PrimitiveTopology, RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType, ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, TexelCopyBufferInfo, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect, TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView, TextureViewDimension, VertexState};
use wgpu::hal::vulkan::conv::map_vk_surface_formats;
use wgpu::LoadOp::Clear;
use wgpu::util::{BufferInitDescriptor, RenderEncoder};
use wgpu::wgc::command::RenderPassErrorInner::Bind;
use wgpu::wgt::{BufferDescriptor, CommandEncoderDescriptor, SamplerDescriptor, TextureDescriptor};
use crate::ffmpeg::decode::{Decoder, DecoderResult};
use crate::ffmpeg::frame::Frame;
use crate::ffmpeg::input::{Input, Stream, StreamType};
use crate::window::app::{AppContext, Scene, State};

pub mod window;
pub mod ffmpeg;

struct VideoSurface {
    texture: Texture,
    view: TextureView,
    sampler: Sampler,
    bind_group: BindGroup,
    frame: Frame,
}

struct PlayerScene {
    render_pipeline: Option<wgpu::RenderPipeline>,
    surface_bind_group_layout: Option<wgpu::BindGroupLayout>,
    vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    video_surface: Option<VideoSurface>,
    begin_time: Instant,
    frame_channel: std::sync::mpsc::Receiver<(Range<f64>, Frame)>,
    video_stream: Option<Stream>,
    video_thread: std::thread::JoinHandle<()>,
    current_frame: Option<(Range<f64>, Frame)>,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2], // NEW!
}

impl Vertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2, // NEW!
                },
            ]
        }
    }
}

impl PlayerScene {
    fn new() -> Self {
        let mut begin_time = SystemTime::now();
        let (sender, receiver) = std::sync::mpsc::sync_channel(3);
        let mut input = Input::open("test.mp4", vec![]).unwrap();
        let stream = input.streams.iter().find(|stream| stream.stream_type == StreamType::Video).unwrap().clone();
        let stream_clone = stream.clone();
        let thread = std::thread::spawn(move || {
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
                            let start = frame.pts.unwrap();
                            let end = start + frame.duration.unwrap();
                            sender.send((start..end, frame.clone())).unwrap();
                            frame.unref();
                        }
                    }
                }
            }
        });
        Self {
            render_pipeline: None,
            video_surface: None,
            frame_channel: receiver,
            video_thread: thread,
            surface_bind_group_layout: None,
            vertex_buffer: None,
            index_buffer: None,
            video_stream: Some(stream_clone),
            begin_time: Instant::now(),
            current_frame: None,
        }
    }
}

impl PlayerScene {
    fn upload_frame(&mut self, state: &mut State, frame: &Frame) {
        if self.video_surface.is_none() {
            let (width, height) = frame.dimensions();
            let texture = state.device.create_texture(&TextureDescriptor {
                label: Some("Video Surface"),
                size: Extent3d {
                    width: width as u32,
                    height: height as u32,
                    depth_or_array_layers: 1
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::R8Unorm,
                usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
                view_formats: &[]
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let sampler = state.device.create_sampler(&SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            });
            let bind_group = state.device.create_bind_group(&BindGroupDescriptor {
                label: None,
                layout: self.surface_bind_group_layout.as_ref().unwrap(),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    }
                ]
            });

            self.video_surface = Some(VideoSurface {
                texture,
                view,
                sampler,
                bind_group,
                frame: Frame::new(),
            })
        }

        let surface = self.video_surface.as_mut().unwrap();
        frame.transfer_hw_data_to(&mut surface.frame, self.video_stream.as_ref().unwrap()).unwrap();
        let frame = &surface.frame;

        state.queue.write_texture(TexelCopyTextureInfo {
            texture: &surface.texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        }, frame.plane(0), TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(frame.plane_stride(0) as u32),
            rows_per_image: Some(frame.height() as u32),
        }, Extent3d {
            width: frame.width() as u32,
            height: frame.height() as u32,
            depth_or_array_layers: 1,
        })
    }

}

impl Scene for PlayerScene {
    fn init(&mut self, state: &mut State, app: &mut AppContext) {
        let shader = state.device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Simple rendering shader"),
            source: ShaderSource::Wgsl(include_str!("res/render.wgsl").into()),
        });
        let surface_bind_group_layout = state.device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                }
            ]
        });
        let pipeline_layout = state.device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Simple rendering pipeline layout"),
            bind_group_layouts: &[Some(&surface_bind_group_layout)],
            immediate_size: 0
        });
        self.surface_bind_group_layout = Some(surface_bind_group_layout);

        let index_buffer = state.device.create_buffer(&BufferDescriptor {
            label: None,
            size: size_of::<u32>() as BufferAddress * 6,
            usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        state.queue.write_buffer(&index_buffer, 0, bytemuck::cast_slice(&[0, 1, 2, 2, 3, 0]));

        let vertex_buffer = state.device.create_buffer(&BufferDescriptor {
            label: None,
            size: size_of::<Vertex>() as BufferAddress * 4,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        state.queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&[
            Vertex { position: [-0.5, -0.5], tex_coords: [0.0, 0.0], },
            Vertex { position: [0.5, -0.5], tex_coords: [1.0, 0.0], },
            Vertex { position: [0.5, 0.5], tex_coords: [1.0, 1.0], },
            Vertex { position: [-0.5, 0.5], tex_coords: [0.0, 1.0], },
        ]));

        self.vertex_buffer = Some(vertex_buffer);
        self.index_buffer = Some(index_buffer);

        let render_pipeline = state.device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Simple rendering pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[Vertex::desc()]
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
                targets: &[Some(ColorTargetState {
                    format: state.config.format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })]
            }),
            multiview_mask: None,
            cache: None
        });

        self.render_pipeline = Some(render_pipeline);
    }

    fn render(&mut self, state: &mut State, app: &mut AppContext, view: &TextureView) {
        let mut encoder = state.device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
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

            if self.current_frame.is_none() {
                if let Some((time_range, frame)) = self.frame_channel.try_recv().ok() {
                    self.current_frame = Some((time_range, frame));
                }
            }
            if let Some((time_range, frame)) = self.current_frame.take() {
                let current = self.begin_time.elapsed().as_secs_f64();
                if current < time_range.start {
                    self.current_frame = Some((time_range, frame));
                } else if current > time_range.end {
                } else {
                    self.upload_frame(state, &frame);
                    self.current_frame = Some((time_range, frame));
                }
            }

            if let Some(video_surface) = self.video_surface.as_ref() {
                pass.set_pipeline(self.render_pipeline.as_ref().unwrap());
                pass.set_bind_group(0, &video_surface.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.as_ref().unwrap().slice(..));
                pass.set_index_buffer(self.index_buffer.as_ref().unwrap().slice(..), IndexFormat::Uint32);
                pass.draw_indexed(0..6, 0, 0..1);
            }
        }

        state.queue.submit(std::iter::once(encoder.finish()));
    }
}

fn main() {

    let mut app = window::app::App::new();
    app.app_context.set_scene(Box::new(PlayerScene::new()));
    app.run().unwrap();
}
