use crate::ffmpeg::decode::{Decoder, DecoderResult};
use crate::ffmpeg::frame::{ColorInfo, Frame};
use crate::ffmpeg::input::{Input, Stream, StreamType};
use crate::window::app::{AppContext, Scene, State};
use std::ops::{Add, Index, Range};
use std::time::Instant;
use wgpu::util::RenderEncoder;
use wgpu::wgt::{BufferDescriptor, CommandEncoderDescriptor, SamplerDescriptor, TextureDescriptor, TextureViewDescriptor};
use wgpu::LoadOp::Clear;
use wgpu::{BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendState, Buffer, BufferAddress, BufferBinding, BufferBindingType, BufferUsages, Color, ColorTargetState, ColorWrites, ComputePass, ComputePassDescriptor, ComputePipeline, ComputePipelineDescriptor, Extent3d, Face, FragmentState, FrontFace, IndexFormat, MultisampleState, Operations, Origin3d, PipelineCompilationOptions, PipelineLayoutDescriptor, PolygonMode, PrimitiveState, PrimitiveTopology, RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType, ShaderModuleDescriptor, ShaderSource, ShaderStages, StorageTextureAccess, StoreOp, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect, TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView, TextureViewDimension, VertexState};

pub mod window;
pub mod ffmpeg;


struct NV12Converter {
    pipeline: ComputePipeline,
    bind_group: BindGroup,
    sampler: Sampler,
    output_texture: Texture,
    y_texture: Texture,
    uv_texture: Texture,
    color_space_buffer: Buffer,
    color_offset_buffer: Buffer,
    width: u32,
    height: u32,
}

impl NV12Converter {
    fn new(state: &mut State, width: u32, height: u32) -> Self {
        let bindgroup_layout = state.device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("NV12 to RGBA bind group layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::StorageTexture {
                        view_dimension: TextureViewDimension::D2,
                        access: StorageTextureAccess::WriteOnly,
                        format: TextureFormat::Rgba8Unorm,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 4,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 5,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None
                    },
                    count: None,
                }
            ]
        });

        let color_space_buffer = state.device.create_buffer(&BufferDescriptor {
            label: Some("NV12 to RGBA color info buffer"),
            size: size_of::<[[f32; 4]; 3]>() as BufferAddress,
            usage: BufferUsages::COPY_DST | BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let color_offset_buffer = state.device.create_buffer(&BufferDescriptor {
            label: Some("NV12 to RGBA color info buffer"),
            size: size_of::<[f32; 3]>() as BufferAddress,
            usage: BufferUsages::COPY_DST | BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });

        let sampler = state.device.create_sampler(&SamplerDescriptor {
            label: Some("NV12 to RGBA sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let output_texture = state.create_simple_2d_texture(
            width,
            height,
            TextureFormat::Rgba8Unorm,
            TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT
        );
        let y_texture = state.create_simple_2d_texture(
            width,
            height,
            TextureFormat::R8Unorm,
            TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING
        );
        let uv_texture = state.create_simple_2d_texture(
            width / 2,
            height / 2,
            TextureFormat::Rg8Unorm,
            TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING
        );

        let layout = state.device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("NV12 to RGBA layout"),
            bind_group_layouts: &[
                Some(&bindgroup_layout),
            ],
            immediate_size: 0
        });

        let bind_group = state.device.create_bind_group(&BindGroupDescriptor {
            label: Some("NV12 to RGBA bindgroup"),
            layout: &bindgroup_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&y_texture.create_view(&TextureViewDescriptor::default()))
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&uv_texture.create_view(&TextureViewDescriptor::default()))
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&output_texture.create_view(&TextureViewDescriptor::default()))
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Sampler(&sampler)
                },
                BindGroupEntry {
                    binding: 4,
                    resource: color_space_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: color_offset_buffer.as_entire_binding(),
                }
            ]
        });

        let module = state.device.create_shader_module(ShaderModuleDescriptor {
            label: Some("NV12 to RGBA shader"),
            source: ShaderSource::Wgsl(include_str!("res/nv12_rgba.wgsl").into()),
        });

        let pipeline = state.device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("NV12 to RGBA compute"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: PipelineCompilationOptions::default(),
            cache: None
        });

        Self {
            pipeline,
            bind_group,
            output_texture,
            y_texture,
            uv_texture,
            width,
            height,
            sampler,
            color_offset_buffer,
            color_space_buffer
        }
    }

    pub fn convert(&mut self, pass: &mut ComputePass, state: &mut State, frame: &Frame) {
        state.queue.write_buffer(&self.color_space_buffer, 0, bytemuck::cast_slice(&[frame.color_space_matrix()]));
        state.queue.write_buffer(&self.color_offset_buffer, 0, bytemuck::cast_slice(&[frame.color_range_vec()]));
        state.queue_simple_2d_texture_write(
            frame.plane(0),
            frame.plane_stride(0),
            &self.y_texture,
            self.width,
            self.height
        );
        state.queue_simple_2d_texture_write(
            frame.plane(1),
            frame.plane_stride(1),
            &self.uv_texture,
            self.width / 2,
            self.height / 2
        );

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.dispatch_workgroups(
            (self.width + 7) / 8,
            (self.height + 7) / 8,
            1
        )
    }
}

struct VideoSurface {
    converter: NV12Converter,
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
    tex_coords: [f32; 2],
}

impl Vertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
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
                    offset: size_of::<[f32; 2]>() as BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ]
        }
    }
}

impl PlayerScene {
    fn new() -> Self {
        let (sender, receiver) = std::sync::mpsc::sync_channel(3);
        let mut input = Input::open("test_o.mp4", vec![]).unwrap();
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
    fn upload_frame(&mut self, state: &mut State, pass: &mut ComputePass, frame: &Frame) -> Option<(u32, u32)> {
        let mut new_dims = None;
        if self.video_surface.is_none() {
            let (width, height) = frame.dimensions();
            let converter = NV12Converter::new(state, frame.width() as u32, frame.height() as u32);
            let view = converter.output_texture.create_view(&wgpu::TextureViewDescriptor::default());
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
                converter,
                view,
                sampler,
                bind_group,
                frame: Frame::new(),
            });

            new_dims = Some((width as u32, height as u32));
        }

        let surface = self.video_surface.as_mut().unwrap();
        frame.transfer_hw_data_to(&mut surface.frame, self.video_stream.as_ref().unwrap()).unwrap();
        let frame = &surface.frame;
        surface.converter.convert(pass, state, frame);

        new_dims
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
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: None,
                timestamp_writes: None
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
                    if let Some((width, height)) = self.upload_frame(state, &mut pass, &frame) {
                        let window_aspect = state.config.width as f32 / state.config.height as f32;
                        let image_aspect = width as f32 / height as f32;

                        let (pw, ph) = if image_aspect > window_aspect {
                            (1.0, window_aspect / image_aspect)
                        } else {
                            (image_aspect / window_aspect, 1.0)
                        };
                        state.queue.write_buffer(self.vertex_buffer.as_ref().unwrap(), 0, bytemuck::cast_slice(&[
                            Vertex { position: [-pw, -ph], tex_coords: [0.0, 0.0], },
                            Vertex { position: [pw, -ph], tex_coords: [1.0, 0.0], },
                            Vertex { position: [pw, ph], tex_coords: [1.0, 1.0], },
                            Vertex { position: [-pw, ph], tex_coords: [0.0, 1.0], },
                        ]));
                    }
                    self.current_frame = Some((time_range, frame));
                }
            }
        }

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
