use wgpu::{Color, Operations, RenderPassColorAttachment, RenderPassDescriptor, StoreOp, TextureView};
use wgpu::LoadOp::Clear;
use wgpu::wgt::CommandEncoderDescriptor;
use crate::window::app::{AppContext, Scene, State};

pub mod window;

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
    let mut app = window::app::App::new();
    app.app_context.set_scene(Box::new(PlayerScene::new()));
    app.run().unwrap();
}
