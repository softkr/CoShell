//! Light drawn on the GPU: the AI's orb and the wave over its conversation.
//!
//! A canvas can only stroke and fill, so light drawn with it looks like line art. Here every
//! pixel works out its own colour: light falls off softly, adds up where colours cross, and
//! nothing has an edge. Without a GPU (the software renderer, and tests) the canvas drawing
//! given as `soft` is shown instead.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Tree, Widget};
use iced_renderer::fallback::Renderer as Either;
use iced_renderer::wgpu::primitive::Renderer as _;
use iced::widget::shader::{self, Viewport};
use iced::wgpu;
use iced::{Element, Length, Rectangle, Renderer, Size, Theme, mouse};
use std::collections::HashMap;

/// What to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Orb,
    /// The wide wave on its own: not shown now, kept for the day it is wanted again.
    #[allow(dead_code)]
    Wave,
}

/// One drawing of light: `at` seconds on the clock, `voice` how much it is speaking (0 the
/// sea, 1 a voice), `surge` extra swell as it arrives (1 → 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    pub kind: Kind,
    pub at: f32,
    pub voice: f32,
    pub surge: f32,
}

impl Light {
    fn values(self) -> [f32; 4] {
        [self.at, self.voice, self.surge, if self.kind == Kind::Orb { 0.0 } else { 1.0 }]
    }
    fn key(self) -> [u32; 4] {
        self.values().map(f32::to_bits)
    }
}

/// The light, `soft` standing in for it where there is no GPU.
pub fn light<'a, Message: 'a>(light: Light, width: Length, height: Length, soft: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    Element::new(Glow { light, width, height, soft: soft.into() })
}

struct Glow<'a, Message> {
    light: Light,
    width: Length,
    height: Length,
    soft: Element<'a, Message>,
}

impl<Message> Widget<Message, Theme, Renderer> for Glow<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.soft)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.soft));
    }
    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let size = limits.resolve(self.width, self.height, Size::ZERO);
        let soft = self.soft.as_widget_mut().layout(&mut tree.children[0], renderer, &layout::Limits::new(Size::ZERO, size));
        layout::Node::with_children(size, vec![soft])
    }
    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        match renderer {
            Either::Primary(gpu) => gpu.draw_primitive(layout.bounds(), self.light),
            Either::Secondary(_) => {
                let soft = layout.children().next().expect("the stand-in");
                self.soft.as_widget().draw(&tree.children[0], renderer, theme, style, soft, cursor, viewport);
            }
        }
    }
}

/// The shader and a pool of settings, one per different drawing in a frame (drawings with
/// the same settings share one).
pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// The target stores linear light: colours are worked out for the eye and turned back.
    linear: bool,
    pool: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    used: HashMap<[u32; 4], usize>,
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("coshell light"), source: wgpu::ShaderSource::Wgsl(include_str!("glow.wgsl").into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("coshell light"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("coshell light"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("coshell light"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Self { pipeline, layout, linear: format.is_srgb(), pool: Vec::new(), used: HashMap::new() }
    }

    fn trim(&mut self) {
        self.used.clear();
    }
}

impl shader::Primitive for Light {
    type Pipeline = Pipeline;

    fn prepare(&self, p: &mut Pipeline, device: &wgpu::Device, queue: &wgpu::Queue, _bounds: &Rectangle, _viewport: &Viewport) {
        if p.used.contains_key(&self.key()) {
            return;
        }
        let slot = p.used.len();
        if slot == p.pool.len() {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("coshell light"), size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("coshell light"), layout: &p.layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }] });
            p.pool.push((buffer, group));
        }
        let mut bytes = [0u8; 32];
        let more = [if p.linear { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0];
        for (i, v) in self.values().into_iter().chain(more).enumerate() {
            bytes[4 * i..4 * i + 4].copy_from_slice(&v.to_ne_bytes());
        }
        queue.write_buffer(&p.pool[slot].0, 0, &bytes);
        p.used.insert(self.key(), slot);
    }

    fn draw(&self, p: &Pipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        if let Some(slot) = p.used.get(&self.key()) {
            pass.set_pipeline(&p.pipeline);
            pass.set_bind_group(0, &p.pool[*slot].1, &[]);
            pass.draw(0..3, 0..1);
        }
        true
    }
}
