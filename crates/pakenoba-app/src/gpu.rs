//! wgpu での箱の描画。
//!
//! egui の画面とは別のテクスチャに描き（4 倍のマルチサンプルと奥行きバッファつき）、
//! 結果を egui に画像として渡す。あとで動画書き出しに転用しやすい形。

use pakenoba_i18n::t;
use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};

use pakenoba_core::math::{Mat4, V3};
use pakenoba_core::template::Vertex;

// egui は「ガンマ補正済みの値がそのまま入ったテクスチャ」を前提にしているので、sRGB 形式にはしない。
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
/// 立体の表示の地の色（PhotoCraft の絵を置く場所と同じ #282828）。
const BACKGROUND: wgpu::Color = wgpu::Color { r: 0.157, g: 0.157, b: 0.157, a: 1.0 };

const SHADER: &str = r#"
struct Globals {
    view_proj: mat4x4<f32>,
    light: vec4<f32>,
    // 抜き穴。xy が中心、zw が半分の幅と高さ（どちらも絵柄の座標）。z が 0 なら穴なし。
    holes: array<vec4<f32>, 2>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var art: texture_2d<f32>;
@group(1) @binding(1) var art_sampler: sampler;
@group(1) @binding(2) var back_art: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) kind: f32,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) kind: f32) -> VertexOut {
    var out: VertexOut;
    out.position = globals.view_proj * vec4<f32>(pos, 1.0);
    out.normal = normal;
    out.uv = uv;
    out.kind = kind;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // 色は光の計算に合う線形の値で扱い、最後に画面用の値へ直す。
    let paper = vec3<f32>(0.87, 0.84, 0.77);
    let inner = vec3<f32>(0.80, 0.76, 0.68);
    let edge = vec3<f32>(0.62, 0.56, 0.47);
    let texel = textureSample(art, art_sampler, in.uv);
    let back = textureSample(back_art, art_sampler, in.uv);
    var base = mix(paper, texel.rgb, texel.a);
    if in.kind > 1.5 {
        base = edge;
    } else if in.kind > 0.5 {
        // 紙の裏（箱の内側の面）。裏に置いた柄が見える。
        base = mix(inner, back.rgb, back.a);
    }
    // 抜き穴の中は描かない。角の丸い四角として判定する。
    for (var i = 0; i < 2; i++) {
        let hole = globals.holes[i];
        if hole.z > 0.0 {
            let q = abs(in.uv - hole.xy) / hole.zw;
            if q.x * q.x * q.x * q.x + q.y * q.y * q.y * q.y < 1.0 {
                discard;
            }
        }
    }
    let n = normalize(in.normal);
    let towards = dot(n, globals.light.xyz);
    let shade = 0.42 + 0.58 * max(towards, 0.0) + 0.10 * max(-towards, 0.0);
    let lit = base * shade;
    return vec4<f32>(pow(lit, vec3<f32>(1.0 / 2.2)), 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: Mat4,
    light: [f32; 4],
    holes: [[f32; 4]; 2],
}

struct Target {
    size: [u32; 2],
    msaa_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    resolve_view: wgpu::TextureView,
}

pub struct BoxRenderer {
    pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    art_layout: wgpu::BindGroupLayout,
    art_bind: wgpu::BindGroup,
    /// 紙の表の絵と、裏の絵。
    front: wgpu::TextureView,
    back: wgpu::TextureView,
    sampler: wgpu::Sampler,
    vertex_buf: wgpu::Buffer,
    vertex_capacity: usize,
    target: Option<Target>,
    egui_id: Option<egui::TextureId>,
}

fn texture(device: &wgpu::Device, label: &str, size: [u32; 2], format: wgpu::TextureFormat, samples: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("box vertices"),
        size: (capacity * std::mem::size_of::<Vertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl BoxRenderer {
    pub fn new(rs: &egui_wgpu::RenderState) -> Self {
        let device = &rs.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("box shader"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("box globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let art_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("box art"),
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
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("box layout"), bind_group_layouts: &[Some(&globals_layout), Some(&art_layout)], immediate_size: 0 });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("box pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32],
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            // 紙は表も裏も見えるので、裏面を捨てない。
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format: COLOR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("box globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("box globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("box art"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let front = Self::upload(rs, 1, 1, &[255, 255, 255, 0]);
        let back = Self::upload(rs, 1, 1, &[255, 255, 255, 0]);
        let art_bind = Self::art_bind_group(rs, &art_layout, &sampler, &front, &back);
        let vertex_capacity = 4096;
        BoxRenderer {
            pipeline,
            globals_buf,
            globals_bind,
            art_layout,
            art_bind,
            front,
            back,
            sampler,
            vertex_buf: vertex_buffer(device, vertex_capacity),
            vertex_capacity,
            target: None,
            egui_id: None,
        }
    }

    /// 画像を GPU に送る。
    fn upload(rs: &egui_wgpu::RenderState, w: u32, h: u32, rgba: &[u8]) -> wgpu::TextureView {
        // 絵柄は sRGB として読ませる（シェーダーには線形の値が届く）。
        let tex = texture(&rs.device, "box art", [w, h], wgpu::TextureFormat::Rgba8UnormSrgb, 1, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        rs.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn art_bind_group(rs: &egui_wgpu::RenderState, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, front: &wgpu::TextureView, back: &wgpu::TextureView) -> wgpu::BindGroup {
        rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("box art"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(front) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(back) },
            ],
        })
    }

    /// 箱に貼る画像を差し替える。`back` が真なら、紙の裏（箱の内側の面）の画像。
    pub fn set_art(&mut self, rs: &egui_wgpu::RenderState, back: bool, w: usize, h: usize, rgba: &[u8]) {
        let max = rs.device.limits().max_texture_dimension_2d as usize;
        if w == 0 || h == 0 || w > max || h > max || rgba.len() != w * h * 4 {
            return;
        }
        let view = Self::upload(rs, w as u32, h as u32, rgba);
        if back {
            self.back = view;
        } else {
            self.front = view;
        }
        self.art_bind = Self::art_bind_group(rs, &self.art_layout, &self.sampler, &self.front, &self.back);
    }

    fn ensure_target(&mut self, rs: &egui_wgpu::RenderState, size: [u32; 2]) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let device = &rs.device;
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let msaa = texture(device, "box msaa", size, COLOR_FORMAT, SAMPLES, attach);
        let depth = texture(device, "box depth", size, DEPTH_FORMAT, SAMPLES, attach);
        let resolve = texture(device, "box resolve", size, COLOR_FORMAT, 1, attach | wgpu::TextureUsages::TEXTURE_BINDING);
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let target = Target { size, msaa_view: view(&msaa), depth_view: view(&depth), resolve_view: view(&resolve) };
        let mut renderer = rs.renderer.write();
        match self.egui_id {
            Some(id) => renderer.update_egui_texture_from_wgpu_texture(device, &target.resolve_view, wgpu::FilterMode::Linear, id),
            None => self.egui_id = Some(renderer.register_native_texture(device, &target.resolve_view, wgpu::FilterMode::Linear)),
        }
        self.target = Some(target);
    }

    /// 箱を描き、egui で表示できる画像の ID を返す。
    pub fn render(&mut self, rs: &egui_wgpu::RenderState, size: [u32; 2], vertices: &[Vertex], view_proj: Mat4, light: V3, holes: [[f32; 4]; 2]) -> Option<egui::TextureId> {
        let max = rs.device.limits().max_texture_dimension_2d;
        let size = [size[0].clamp(2, max), size[1].clamp(2, max)];
        self.ensure_target(rs, size);
        if vertices.len() > self.vertex_capacity {
            self.vertex_capacity = vertices.len().next_power_of_two();
            self.vertex_buf = vertex_buffer(&rs.device, self.vertex_capacity);
        }
        rs.queue.write_buffer(&self.vertex_buf, 0, bytemuck::cast_slice(vertices));
        rs.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&Globals { view_proj, light: [light[0], light[1], light[2], 0.0], holes }));

        let target = self.target.as_ref()?;
        let mut encoder = rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("box") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("box"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa_view,
                    resolve_target: Some(&target.resolve_view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(BACKGROUND), store: wgpu::StoreOp::Discard },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.globals_bind, &[]);
            pass.set_bind_group(1, &self.art_bind, &[]);
            pass.set_vertex_buffer(0, self.vertex_buf.slice(..std::mem::size_of_val(vertices) as u64));
            pass.draw(0..vertices.len() as u32, 0..1);
        }
        rs.queue.submit(Some(encoder.finish()));
        self.egui_id
    }

    /// 箱を、画面とは別の画像に描いて、RGBA（アルファは乗算なし）で読み戻す。画像や動画に書き出すとき用。
    /// `background` のアルファを 0 にすると、背景は透明になる。
    #[allow(clippy::too_many_arguments)]
    pub fn capture(&mut self, rs: &egui_wgpu::RenderState, size: [u32; 2], vertices: &[Vertex], view_proj: Mat4, light: V3, holes: [[f32; 4]; 2], background: [f32; 4]) -> Result<Vec<u8>, String> {
        let max = rs.device.limits().max_texture_dimension_2d;
        if size[0] < 2 || size[1] < 2 || size[0] > max || size[1] > max {
            return Err(t!("This PC cannot draw a {} x {} px image (up to {max} px)", size[0], size[1]));
        }
        let device = &rs.device;
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let msaa = texture(device, "capture msaa", size, COLOR_FORMAT, SAMPLES, attach);
        let depth = texture(device, "capture depth", size, DEPTH_FORMAT, SAMPLES, attach);
        let resolve = texture(device, "capture resolve", size, COLOR_FORMAT, 1, attach | wgpu::TextureUsages::COPY_SRC);
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        if vertices.len() > self.vertex_capacity {
            self.vertex_capacity = vertices.len().next_power_of_two();
            self.vertex_buf = vertex_buffer(device, self.vertex_capacity);
        }
        rs.queue.write_buffer(&self.vertex_buf, 0, bytemuck::cast_slice(vertices));
        rs.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&Globals { view_proj, light: [light[0], light[1], light[2], 0.0], holes }));
        // 1 行の長さは 256 バイトの倍数にそろえる（wgpu の決まり）。
        let row = (size[0] * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor { label: Some("capture readback"), size: (row * size[1]) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("capture") });
        {
            let (msaa_view, depth_view, resolve_view) = (view(&msaa), view(&depth), view(&resolve));
            let clear = wgpu::Color { r: background[0] as f64, g: background[1] as f64, b: background[2] as f64, a: background[3] as f64 };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("capture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &msaa_view,
                    resolve_target: Some(&resolve_view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Discard },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.globals_bind, &[]);
            pass.set_bind_group(1, &self.art_bind, &[]);
            pass.set_vertex_buffer(0, self.vertex_buf.slice(..std::mem::size_of_val(vertices) as u64));
            pass.draw(0..vertices.len() as u32, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &resolve, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size[1]) } },
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        );
        rs.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        let (send, receive) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| t!("Could not wait for drawing to finish: {e}"))?;
        receive.recv().map_err(|_| t!("Could not read the picture back").to_owned())?.map_err(|e| t!("Could not read the picture back: {e}"))?;
        let mapped = slice.get_mapped_range().map_err(|e| t!("Could not read the picture back: {e:?}"))?;
        let mut rgba = Vec::with_capacity((size[0] * size[1] * 4) as usize);
        for y in 0..size[1] as usize {
            let start = y * row as usize;
            rgba.extend_from_slice(&mapped[start..start + size[0] as usize * 4]);
        }
        drop(mapped);
        readback.unmap();
        // 色は乗算済みで描かれる（背景が透明のとき）ので、乗算なしに直す。
        if background[3] < 1.0 {
            for p in rgba.chunks_exact_mut(4) {
                let a = p[3];
                if a > 0 && a < 255 {
                    for c in &mut p[..3] {
                        *c = ((*c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                    }
                }
            }
        }
        Ok(rgba)
    }
}
