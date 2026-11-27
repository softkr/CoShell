//! Frosted glass, as macOS draws it: a panel through which what is behind it shows, blurred.
//!
//! The panel's content is drawn on a layer of its own; under it, a primitive copies the part of
//! the frame already drawn behind the panel, makes it small, blurs it both ways and draws it
//! back in the panel's rounded shape (`glass.wgsl`). Reading the frame needs a surface that can
//! be copied from (see `vendor/iced_wgpu/COSHELL_PATCH.md`); where it cannot, or without a GPU,
//! only the content is drawn, and `working()` says so, so the panel can keep a solid ground.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::widget::shader::{self, Viewport};
use iced::wgpu;
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector, mouse};
use iced_renderer::fallback::Renderer as Either;
use iced_renderer::wgpu::primitive::Renderer as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The glass has been drawn: the frame could be read. Until then panels keep a solid ground.
static WORKING: AtomicBool = AtomicBool::new(false);

/// Whether frosted glass is being drawn on this machine.
pub fn working() -> bool {
    WORKING.load(Ordering::Relaxed)
}

/// `content` on frosted glass with corners of `radius`, the glass showing `strength` of the way
/// (0: not at all, 1: fully).
pub fn glass<'a, Message: 'a>(content: impl Into<Element<'a, Message>>, radius: f32, strength: f32) -> Element<'a, Message> {
    Element::new(Glass { content: content.into(), radius, strength })
}

struct Glass<'a, Message> {
    content: Element<'a, Message>,
    radius: f32,
    strength: f32,
}

impl<Message> Widget<Message, Theme, Renderer> for Glass<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let content = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(content.size(), vec![content])
    }
    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        use iced::advanced::Renderer as _;
        let content = layout.children().next().expect("the content");
        if self.strength > 0.0
            && let Either::Primary(gpu) = renderer
        {
            gpu.draw_primitive(layout.bounds(), Pane { radius: self.radius, strength: self.strength.min(1.0) });
        }
        // On a layer of its own, so the glass is under all of it (its shadow too).
        renderer.with_layer(*viewport, |r| self.content.as_widget().draw(&tree.children[0], r, theme, style, content, cursor, viewport));
    }
    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        let content = layout.children().next().expect("the content");
        self.content.as_widget_mut().update(&mut tree.children[0], event, content, cursor, renderer, clipboard, shell, viewport);
    }
    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        let content = layout.children().next().expect("the content");
        self.content.as_widget().mouse_interaction(&tree.children[0], content, cursor, viewport, renderer)
    }
    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        let content = layout.children().next().expect("the content");
        self.content.as_widget_mut().operate(&mut tree.children[0], content, renderer, operation);
    }
    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, Renderer>> {
        let content = layout.children().next().expect("the content");
        self.content.as_widget_mut().overlay(&mut tree.children[0], content, renderer, viewport, translation)
    }
}

/// One pane of glass.
#[derive(Debug, Clone, Copy)]
struct Pane {
    radius: f32,
    strength: f32,
}

/// What a frame's pane needs at render time: where it is (physical px) and its textures.
struct Queued {
    rect: [u32; 4],
    size: (u32, u32),
}

/// The textures and settings for a pane of one size.
struct Job {
    src: wgpu::Texture,
    /// Down to a half, then a quarter; blurred across into `across`, then down back into `quarter`.
    half: wgpu::TextureView,
    quarter: wgpu::TextureView,
    across: wgpu::TextureView,
    passes: [(wgpu::Buffer, wgpu::BindGroup); 5],
}

pub struct Pipeline {
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    down: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    paint: wgpu::RenderPipeline,
    jobs: std::collections::HashMap<(u32, u32), Job>,
    queued: Vec<Queued>,
    next: AtomicUsize,
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("coshell glass"), source: wgpu::ShaderSource::Wgsl(include_str!("glass.wgsl").into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("coshell glass"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("coshell glass"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let make = |entry: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("coshell glass"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("coshell glass"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            format,
            down: make("fs_down", None),
            blur: make("fs_blur", None),
            paint: make("fs_glass", Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)),
            layout,
            sampler,
            jobs: std::collections::HashMap::new(),
            queued: Vec::new(),
            next: AtomicUsize::new(0),
        }
    }

    fn trim(&mut self) {
        self.queued.clear();
        self.next.store(0, Ordering::Relaxed);
    }
}

impl Pipeline {
    fn job(&mut self, device: &wgpu::Device, (w, h): (u32, u32)) -> &mut Job {
        let (format, layout, sampler) = (self.format, &self.layout, &self.sampler);
        self.jobs.entry((w, h)).or_insert_with(|| {
            let texture = |w: u32, h: u32, copy: bool| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("coshell glass"),
                    size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | if copy { wgpu::TextureUsages::COPY_DST } else { wgpu::TextureUsages::RENDER_ATTACHMENT },
                    view_formats: &[],
                })
            };
            let src = texture(w, h, true);
            let half = texture(w / 2, h / 2, false).create_view(&Default::default());
            let quarter = texture(w / 4, h / 4, false).create_view(&Default::default());
            let across = texture(w / 4, h / 4, false).create_view(&Default::default());
            let src_view = src.create_view(&Default::default());
            let pass = |from: &wgpu::TextureView| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("coshell glass"), size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("coshell glass"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(from) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                        wgpu::BindGroupEntry { binding: 2, resource: buffer.as_entire_binding() },
                    ],
                });
                (buffer, group)
            };
            // Reading: the copy, the half, the quarter (across), what was blurred across (back
            // down), and the quarter again (painted).
            let passes = [pass(&src_view), pass(&half), pass(&quarter), pass(&across), pass(&quarter)];
            Job { src, half, quarter, across, passes }
        })
    }
}

fn values(v: [f32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, x) in v.into_iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_ne_bytes());
    }
    out
}

impl shader::Primitive for Pane {
    type Pipeline = Pipeline;

    fn prepare(&self, p: &mut Pipeline, device: &wgpu::Device, queue: &wgpu::Queue, bounds: &Rectangle, viewport: &Viewport) {
        let scale = viewport.scale_factor();
        let screen = viewport.physical_size();
        let x0 = (bounds.x * scale).round().clamp(0.0, screen.width as f32) as u32;
        let y0 = (bounds.y * scale).round().clamp(0.0, screen.height as f32) as u32;
        let x1 = ((bounds.x + bounds.width) * scale).round().clamp(0.0, screen.width as f32) as u32;
        let y1 = ((bounds.y + bounds.height) * scale).round().clamp(0.0, screen.height as f32) as u32;
        let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        p.queued.push(Queued { rect: [x0, y0, w, h], size: (w, h) });
        if w < 8 || h < 8 {
            return;
        }
        let job = p.job(device, (w, h));
        let texel = |w: u32, h: u32| [1.0 / w.max(1) as f32, 1.0 / h.max(1) as f32];
        let [sx, sy] = texel(w, h);
        let [hx, hy] = texel(w / 2, h / 2);
        let [qx, qy] = texel(w / 4, h / 4);
        let shape = [self.radius * scale, self.strength, w as f32, h as f32];
        let writes = [
            [sx, sy, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            [hx, hy, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            [qx, qy, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            [qx, qy, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
            [qx, qy, 0.0, 0.0, shape[0], shape[1], shape[2], shape[3]],
        ];
        for ((buffer, _), v) in job.passes.iter().zip(writes) {
            queue.write_buffer(buffer, 0, &values(v));
        }
    }

    fn render(&self, p: &Pipeline, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, clip: &Rectangle<u32>) {
        let Some(q) = p.queued.get(p.next.fetch_add(1, Ordering::Relaxed)) else { return };
        let frame = target.texture();
        if !frame.usage().contains(wgpu::TextureUsages::COPY_SRC) || frame.format() != p.format {
            return;
        }
        let Some(job) = p.jobs.get(&q.size).filter(|_| q.size.0 >= 8 && q.size.1 >= 8) else { return };
        let [x, y, w, h] = q.rect;
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: frame, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &job.src, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let step = |encoder: &mut wgpu::CommandEncoder, into: &wgpu::TextureView, pipeline: &wgpu::RenderPipeline, group: &wgpu::BindGroup| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("coshell glass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: into,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        };
        step(encoder, &job.half, &p.down, &job.passes[0].1);
        step(encoder, &job.quarter, &p.down, &job.passes[1].1);
        step(encoder, &job.across, &p.blur, &job.passes[2].1);
        step(encoder, &job.quarter, &p.blur, &job.passes[3].1);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("coshell glass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        pass.set_scissor_rect(clip.x, clip.y, clip.width, clip.height);
        pass.set_pipeline(&p.paint);
        pass.set_bind_group(0, &job.passes[4].1, &[]);
        pass.draw(0..3, 0..1);
        WORKING.store(true, Ordering::Relaxed);
    }
}
