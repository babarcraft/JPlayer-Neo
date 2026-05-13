use std::sync::Arc;
use egui_wgpu::RendererOptions;
use ffmpeg_sys_next::AVPixelFormat::{AV_PIX_FMT_RGB24, AV_PIX_FMT_YUV420P};
use wgpu::wgt::{CommandEncoderDescriptor, DeviceDescriptor, TextureDescriptor};
use wgpu::{Color, ExperimentalFeatures, Extent3d, Label, LoadOp, MemoryHints, Operations, Origin3d, RenderPassColorAttachment, RenderPassDescriptor, StoreOp, SurfaceTexture, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect, TextureDimension, TextureFormat, TextureUsages, TextureView, Trace};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

pub trait Scene {
    fn init(&mut self, state: &mut State, app: &mut AppContext);
    fn render(&mut self, state: &mut State, app: &mut AppContext, view: &TextureView);
}

pub struct State {
    pub window: Arc<Window>,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub egui_context: egui::Context,
    pub egui_state: egui_winit::State,
    pub egui_renderer: egui_wgpu::Renderer,
    surface_configured: bool
}

impl State {
    async fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: Default::default(),
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            display: None,
        });

        let surface = instance.create_surface(window.clone())?;
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }).await?;

        let (device, queue) = adapter.request_device(&DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::MAPPABLE_PRIMARY_BUFFERS,
            required_limits: wgpu::Limits::default(),
            experimental_features: ExperimentalFeatures::disabled(),
            memory_hints: MemoryHints::default(),
            trace: Trace::Off
        }).await?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats.iter()
            .find(|f| f.is_srgb())
            .copied().unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: window.inner_size().width,
            height: window.inner_size().height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2
        };

        let egui_context = egui::Context::default();
        let id = egui_context.viewport_id();
        let egui_state = egui_winit::State::new(egui_context.clone(), id, &window, None, None, None);
        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, RendererOptions::default());

        Ok(State {
            window: window.clone(),
            adapter,
            device,
            queue,
            surface,
            config,
            surface_configured: false,
            egui_state,
            egui_renderer,
            egui_context,
        })
    }

    pub fn create_simple_2d_texture(&mut self, width: u32, height: u32, format: TextureFormat, usage: TextureUsages) -> Texture {
        self.device.create_texture(&TextureDescriptor {
            label: None,
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage,
            view_formats: &[]
        })
    }

    pub fn queue_simple_2d_texture_write(&mut self, data: &[u8], stride: usize, texture: &Texture, width: u32, height: u32) {
        self.queue.write_texture(TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        }, data, TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride as u32),
            rows_per_image: Some(height),
        }, Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        })
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
            self.surface_configured = true
        }
    }

    fn current(&self) -> anyhow::Result<Option<(SurfaceTexture, TextureView)>> {
        if !self.surface_configured {
            return Ok(None)
        }

        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(surface_texture) => surface_texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => {
                self.surface.configure(&self.device, &self.config);
                surface_texture
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => {
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                anyhow::bail!("Lost device");
            }
        };

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(Some((output, view)))
    }
}

pub struct AppContext {
    scene: Option<Box<dyn Scene>>,
    scene_stack: Vec<Box<dyn Scene>>,
}

impl AppContext {
    fn new() -> Self {
        Self {
            scene: None,
            scene_stack: Vec::new(),
        }
    }

    pub fn set_scene(&mut self, scene: Box<dyn Scene>) {
        self.scene = Some(scene);
    }

    pub fn pop_scene(&mut self) {
        self.scene = Some(self.scene_stack.pop().unwrap());
    }

    pub fn push_scene(&mut self) {
        self.scene_stack.push(self.scene.take().unwrap());
    }
}

pub struct App {
    window: Option<Arc<Window>>,
    pub app_context: AppContext,
    state: Option<State>,
}

impl App {
    pub fn new() -> Self {
        Self { window: None, state: None, app_context: AppContext::new() }
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        let event_loop = EventLoop::new()?;
        event_loop.run_app(self)?;
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = WindowAttributes::default()
            .with_title("Test")
            .with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0));
        let window = Arc::new(event_loop.create_window(attributes).unwrap());
        self.window = Some(window.clone());
        let mut state = pollster::block_on(State::new(window)).unwrap();
        if let Some(mut scene) = self.app_context.scene.take() {
            scene.init(&mut state, &mut self.app_context);
            self.app_context.scene = Some(scene);
        }
        self.state = Some(state);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(size) => {
                let state = match &mut self.state {
                    Some(state) => state,
                    None => return
                };
                state.resize(size);
            }

            WindowEvent::RedrawRequested => {
                let state = match &mut self.state {
                    Some(state) => state,
                    None => return
                };
                let window = match &mut self.window {
                    Some(window) => window,
                    None => return
                };
                let context = &mut self.app_context;

                window.request_redraw();

                match state.current() {
                    Ok(Some((surface, view))) => {
                        let mut scene = match context.scene.take() {
                            Some(scene) => scene,
                            None => return
                        };
                        
                        scene.render(state, context, &view);
                        surface.present();

                        if let Some(_) = context.scene.as_ref() {
                            context.scene_stack.push(scene);
                        } else {
                            context.scene = Some(scene);
                        }
                    }
                    Err(err) => {
                        println!("Failed: {:?}", err);
                    }
                    _ => {}
                }
            }

            WindowEvent::RedrawRequested => {
            }

            _ => {}
        }
    }
}
