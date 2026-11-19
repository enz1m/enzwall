#![allow(
    unsafe_code,
    reason = "OpenGL ES exposes stateful unsafe FFI under the current EGL context."
)]
use enzwall_scene::{
    Align, Blend, Evaluated, Fit, Kind, Particles, Property, Props, Rgba, Scene, WaveFill,
};
use glow::HasContext;

const VERTEX: &str = r#"#version 300 es
layout(location = 0) in vec2 a_pos;
uniform mat3 u_xform;
out vec2 v_uv;
void main() {
    v_uv = a_pos;
    vec3 p = u_xform * vec3(a_pos, 1.0);
    gl_Position = vec4(p.xy, 0.0, 1.0);
}
"#;

const FRAGMENT: &str = r#"#version 300 es
precision highp float;
in vec2 v_uv;
out vec4 o_color;
uniform int u_kind;
uniform vec4 u_color;
uniform vec4 u_color2;
uniform vec4 u_stops[4];
uniform int u_count;
uniform vec4 u_params;
uniform vec4 u_rect;
uniform vec4 u_uv;
uniform float u_opacity;
uniform float u_aspect;
uniform float u_samples[64];
uniform sampler2D u_tex;

float hash(vec2 p) { return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715)))); }

vec4 ramp(float t) {
    t = clamp(t, 0.0, 1.0);
    float seg = t * float(u_count - 1);
    int i = int(min(floor(seg), float(u_count - 2)));
    return mix(u_stops[i], u_stops[i + 1], seg - float(i));
}

float lines(float coordinate, float width) {
    float d = abs(fract(coordinate + 0.5) - 0.5) / max(fwidth(coordinate), 1e-4);
    return 1.0 - smoothstep(width * 0.5, width * 0.5 + 1.0, d);
}

void main() {
    vec4 c = u_color;
    float a = 1.0;
    if (u_kind == 0) {
        if (u_params.x > 0.0) {
            vec2 size = u_params.yz;
            vec2 q = abs((v_uv - 0.5) * size) - (size * 0.5 - u_params.x);
            float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - u_params.x;
            a = 1.0 - smoothstep(-0.75, 0.75, d);
        }
    } else if (u_kind == 1) {
        vec2 d = vec2(cos(u_params.x), sin(u_params.x));
        float t = 0.5 + dot(v_uv - 0.5, d) / (abs(d.x) + abs(d.y));
        c = ramp(t);
    } else if (u_kind == 2) {
        c = ramp(length((v_uv - 0.5) * vec2(u_aspect, 1.0)) * 2.0 / length(vec2(u_aspect, 1.0)));
    } else if (u_kind == 3) {
        float d = length(v_uv * 2.0 - 1.0);
        a = 1.0 - smoothstep(1.0 - u_params.x, 1.0, d);
        a = a * a * (3.0 - 2.0 * a);
    } else if (u_kind == 4) {
        vec2 q = v_uv * 2.0 - 1.0;
        float d = length(q);
        float half_width = u_params.x * 0.5;
        float dist = abs(d - (1.0 - half_width));
        // Most of a ring's square is far from its band; every point an arc
        // or its round caps can cover lies within half a width of the band's
        // centre line, so the rest is dropped before blending. Only the ring
        // program discards: on Intel a discard anywhere in a program slows
        // every draw made with it.
#ifdef RING
        if (dist >= half_width) discard;
#endif
        a = 1.0 - smoothstep(half_width * (1.0 - u_params.y), half_width, dist);
        if (u_params.z < 0.9999 && u_count == 1) {
            float turn = fract(atan(q.x, -q.y) / 6.2831853 - u_params.w);
            float px = max(fwidth(turn), 1e-5);
            float inside = min(turn, u_params.z - turn) / px;
            a *= clamp(inside + 0.5, 0.0, 1.0);
        } else if (u_params.z < 0.9999) {
            float turn = fract(atan(q.x, -q.y) / 6.2831853 - u_params.w);
            if (u_params.z <= 0.0 || turn > u_params.z) {
                float centre = 1.0 - half_width;
                float a0 = 6.2831853 * u_params.w;
                float a1 = 6.2831853 * (u_params.w + max(u_params.z, 0.0));
                float cap = min(length(q - centre * vec2(sin(a0), -cos(a0))),
                                length(q - centre * vec2(sin(a1), -cos(a1))));
                a = 1.0 - smoothstep(half_width * (1.0 - u_params.y), half_width, cap);
            }
        }
    } else if (u_kind == 5) {
        vec2 p = u_rect.xy + v_uv * u_rect.zw;
        float x = p.x - u_params.w;
        float wave = sin(6.2831853 * (u_params.y * x + u_uv.x))
                   + u_params.z * sin(6.2831853 * (u_params.y * 2.37 * x + u_uv.x * 2.0) + 1.3);
        float curve = u_uv.y + u_params.x * wave / (1.0 + u_params.z);
        float edge = u_uv.z;
        float dy = p.y - curve;
        if (u_uv.w > 2.5) {
            a = dy < 0.0 ? 1.0 - smoothstep(0.0, edge, -dy) : 1.0 - smoothstep(0.0, edge * 0.08, dy);
            a *= a;
        } else if (u_uv.w > 1.5) {
            a = 1.0 - smoothstep(0.0, edge, abs(dy));
            a *= a;
        } else if (u_uv.w > 0.5) {
            a = 1.0 - smoothstep(-edge, edge, dy);
            if (u_count < 0) c = mix(vec4(u_color2.rgb, c.a), c, clamp((p.y - u_rect.y) / max(curve - u_rect.y, 1e-4), 0.0, 1.0));
        } else {
            a = smoothstep(-edge, edge, dy);
            // The shade runs to the fill's own bottom (u_color2.a), which a
            // nearer wave may hide below the drawn quad.
            float bottom = u_color2.a;
            if (u_count < 0) c = mix(c, vec4(u_color2.rgb, c.a), clamp(dy / max(bottom - curve, 1e-4), 0.0, 1.0));
        }
    } else if (u_kind == 6) {
        vec4 t = texture(u_tex, u_uv.xy + v_uv * u_uv.zw);
        o_color = t * u_opacity;
        return;
    } else if (u_kind == 7) {
        a = texture(u_tex, v_uv).r;
    } else if (u_kind == 8) {
        float y = max(v_uv.y, 0.001);
        float z = 1.0 / y;
        float across = lines(z * u_params.x * 0.1 - u_uv.x, u_params.z);
        float along = lines(((v_uv.x - 0.5) * z - u_uv.y) * u_params.y * 0.25, u_params.z);
        a = max(across, along) * smoothstep(0.0, u_params.w + 0.001, y);
    } else if (u_kind == 9) {
        float last = float(u_count - 1);
        float s = v_uv.x * (last - 1.0) + u_params.w;
        int i = int(floor(s));
        float f = fract(s);
        float v0 = u_samples[clamp(i, 0, u_count - 1)];
        float v1 = u_samples[clamp(i + 1, 0, u_count - 1)];
        float v = mix(v0, v1, f * f * (3.0 - 2.0 * f));
        float curve = 1.0 - v;
        float per_px = max(fwidth(v_uv.y), 1e-5);
        float slope = -(v1 - v0) * 6.0 * f * (1.0 - f) * (last - 1.0) / max(u_aspect, 1e-3);
        float vertical = (v_uv.y - curve) / per_px;
        float dpx = vertical / sqrt(1.0 + slope * slope);
        float line = 1.0 - smoothstep(u_params.x * 0.5, u_params.x * 0.5 + 1.0, abs(dpx));
        float glow = u_params.y * exp(-abs(vertical) / (4.0 + 18.0 * u_params.y)) * 0.55;
        float fill = v_uv.y > curve ? 1.0 - (v_uv.y - curve) / max(1.0 - curve, 1e-3) : 0.0;
        float la = clamp(line + glow, 0.0, 1.0) * c.a;
        float fa = fill * u_color2.a;
        o_color = vec4(c.rgb * la + u_color2.rgb * fa * (1.0 - la), la + fa * (1.0 - la)) * u_opacity;
        return;
    }
    float alpha = c.a * a * u_opacity;
    vec3 rgb = c.rgb + (hash(gl_FragCoord.xy) - 0.5) / 255.0;
    o_color = vec4(rgb * alpha, alpha);
}
"#;

pub(crate) struct Texture {
    pub id: glow::Texture,
    pub width: u32,
    pub height: u32,
}

pub(crate) struct TextTextures {
    pub fill: Texture,
    pub outline: Option<Texture>,
    pub shadow: Option<Texture>,
    /// Padding added around the glyphs for outline and shadow, in pixels.
    pub pad: f32,
}

/// Samples oldest to newest (0–1) and how far the newest step has scrolled in.
pub(crate) struct GraphData {
    pub samples: Vec<f32>,
    pub scroll: f32,
}

pub(crate) struct Resources<'a> {
    pub images: &'a [Option<Texture>],
    pub texts: &'a [Option<TextTextures>],
    pub gauges: &'a [Option<f32>],
    pub graphs: &'a [Option<GraphData>],
}

pub(crate) struct Renderer {
    pub gl: glow::Context,
    program: glow::Program,
    /// The same shader with ring fragments outside the band discarded, used
    /// for rings only; the main program when it does not compile.
    ring: Option<(glow::Program, Locations)>,
    /// Whether the ring program is the one in use.
    ring_bound: std::cell::Cell<bool>,
    vao: glow::VertexArray,
    _vbo: glow::Buffer,
    loc: Locations,
}

struct Locations {
    xform: Option<glow::UniformLocation>,
    kind: Option<glow::UniformLocation>,
    color: Option<glow::UniformLocation>,
    color2: Option<glow::UniformLocation>,
    stops: Option<glow::UniformLocation>,
    count: Option<glow::UniformLocation>,
    params: Option<glow::UniformLocation>,
    rect: Option<glow::UniformLocation>,
    uv: Option<glow::UniformLocation>,
    opacity: Option<glow::UniformLocation>,
    aspect: Option<glow::UniformLocation>,
    samples: Option<glow::UniformLocation>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bounds {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Bounds {
    pub(crate) fn union(self, other: Self) -> Self {
        let (x0, y0) = (self.x.min(other.x), self.y.min(other.y));
        let (x1, y1) = (
            (self.x + self.w).max(other.x + other.w),
            (self.y + self.h).max(other.y + other.h),
        );
        Self {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }
}

pub(crate) type Area = Option<Bounds>;

fn grow(area: &mut Area, b: Bounds) {
    *area = Some(area.map_or(b, |a| a.union(b)));
}

fn rgba(c: Rgba) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

// Quad coordinates are pixels; pivot coordinates are box fractions.
struct Quad {
    origin: (f32, f32),
    size: (f32, f32),
    pivot: (f32, f32),
    scale: f32,
    rotation: f32,
    kind: i32,
    color: [f32; 4],
    color2: [f32; 4],
    params: [f32; 4],
    rect: [f32; 4],
    uv: [f32; 4],
    opacity: f32,
    count: i32,
    blend: Blend,
}

impl Quad {
    fn new(origin: (f32, f32), size: (f32, f32), kind: i32, blend: Blend) -> Self {
        Self {
            origin,
            size,
            pivot: (0.5, 0.5),
            scale: 1.0,
            rotation: 0.0,
            kind,
            color: [1.0; 4],
            color2: [0.0; 4],
            params: [0.0; 4],
            rect: [0.0; 4],
            uv: [0.0, 0.0, 1.0, 1.0],
            opacity: 1.0,
            count: 0,
            blend,
        }
    }

    fn centred(centre: (f32, f32), size: (f32, f32), kind: i32, blend: Blend) -> Self {
        Self::new(
            (centre.0 - size.0 / 2.0, centre.1 - size.1 / 2.0),
            size,
            kind,
            blend,
        )
    }
}

fn quad_bounds(q: &Quad) -> Bounds {
    let pivot = (
        q.origin.0 + q.size.0 * q.pivot.0,
        q.origin.1 + q.size.1 * q.pivot.1,
    );
    let (sin, cos) = q.rotation.to_radians().sin_cos();
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (cx, cy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let rx = (q.origin.0 + q.size.0 * cx - pivot.0) * q.scale;
        let ry = (q.origin.1 + q.size.1 * cy - pivot.1) * q.scale;
        let (x, y) = (pivot.0 + cos * rx - sin * ry, pivot.1 + sin * rx + cos * ry);
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    Bounds {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}

fn hash(index: u32, seed: u32, salt: u32) -> f32 {
    let mut x = index.wrapping_mul(0x9E37_79B9)
        ^ seed.wrapping_mul(0x85EB_CA6B)
        ^ salt.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// How far down (a fraction of the height) the layers in front of `index`
/// leave anything visible: an opaque wave filled downwards covers every row
/// below its lowest point, so a wave behind it need not be drawn there.
fn hidden_below(scene: &Scene, state: &Evaluated, index: usize) -> f32 {
    scene
        .layers
        .iter()
        .zip(&state.layers)
        .skip(index + 1)
        .filter_map(|(layer, st)| match &layer.kind {
            Kind::Wave {
                color,
                amplitude,
                softness,
                fill: WaveFill::Down,
                ..
            } if layer.blend == Blend::Normal && color.a >= 1.0 && st.props.opacity >= 1.0 => {
                // The curve stays within one amplitude of its line and the
                // fill is solid one softness below it.
                Some(st.props.y + amplitude + softness)
            }
            _ => None,
        })
        .fold(1.0, f32::min)
}

/// Compiles the shared vertex shader with `fragment` into a program.
///
/// # Safety
/// The caller's EGL context must be current.
unsafe fn link_program(gl: &glow::Context, fragment: &str) -> Result<glow::Program, String> {
    // SAFETY: the caller's EGL context is current; the objects made here are owned by the renderer.
    unsafe {
        let program = gl.create_program()?;
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(format!("shader: {}", gl.get_shader_info_log(shader)));
            }
            gl.attach_shader(program, shader);
            gl.delete_shader(shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(format!("program: {}", gl.get_program_info_log(program)));
        }
        Ok(program)
    }
}

/// The uniform locations of a program made by [`link_program`].
///
/// # Safety
/// The caller's EGL context must be current.
unsafe fn locations(gl: &glow::Context, program: glow::Program) -> Locations {
    // SAFETY: the caller's EGL context is current and `program` is linked.
    let at = |name| unsafe { gl.get_uniform_location(program, name) };
    Locations {
        xform: at("u_xform"),
        kind: at("u_kind"),
        color: at("u_color"),
        color2: at("u_color2"),
        stops: at("u_stops"),
        count: at("u_count"),
        params: at("u_params"),
        rect: at("u_rect"),
        uv: at("u_uv"),
        opacity: at("u_opacity"),
        aspect: at("u_aspect"),
        samples: at("u_samples"),
    }
}

impl Renderer {
    pub(crate) fn new(gl: glow::Context) -> Result<Self, String> {
        // SAFETY: the renderer owns these GL objects; its EGL context is current throughout the operation.
        unsafe {
            let program = link_program(&gl, FRAGMENT)?;
            let ring_source =
                FRAGMENT.replacen("#version 300 es\n", "#version 300 es\n#define RING\n", 1);
            let ring = link_program(&gl, &ring_source).ok().map(|ring| {
                gl.use_program(Some(ring));
                gl.uniform_1_i32(gl.get_uniform_location(ring, "u_tex").as_ref(), 0);
                (ring, locations(&gl, ring))
            });
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let quad: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                &quad.map(f32::to_ne_bytes).concat(),
                glow::STATIC_DRAW,
            );
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            let loc = locations(&gl, program);
            gl.use_program(Some(program));
            gl.uniform_1_i32(gl.get_uniform_location(program, "u_tex").as_ref(), 0);
            gl.enable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            Ok(Self {
                gl,
                program,
                ring,
                ring_bound: std::cell::Cell::new(false),
                vao,
                _vbo: vbo,
                loc,
            })
        }
    }

    /// Uploads premultiplied RGBA8 or R8 coverage; non-repeating axes clamp at the edge.
    pub(crate) fn upload(
        &self,
        width: u32,
        height: u32,
        pixels: &[u8],
        alpha_only: bool,
        repeat: (bool, bool),
    ) -> Result<Texture, String> {
        // SAFETY: the renderer owns these GL objects; its EGL context is current throughout the operation.
        unsafe {
            let gl = &self.gl;
            let id = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(id));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            let (internal, format) = if alpha_only {
                (glow::R8, glow::RED)
            } else {
                (glow::RGBA8, glow::RGBA)
            };
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal as i32,
                width as i32,
                height as i32,
                0,
                format,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(pixels)),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            let wrap = |on: bool| if on { glow::REPEAT } else { glow::CLAMP_TO_EDGE } as i32;
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap(repeat.0));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap(repeat.1));
            Ok(Texture { id, width, height })
        }
    }

    pub(crate) fn delete(&self, texture: Texture) {
        // SAFETY: the texture is owned by this renderer and its EGL context is current.
        unsafe { self.gl.delete_texture(texture.id) }
    }

    pub(crate) fn capture(
        &self,
        width: u32,
        height: u32,
        draw: impl FnOnce(&Self),
    ) -> Result<Texture, String> {
        let texture = self.upload(
            width,
            height,
            &vec![0; (width * height * 4) as usize],
            false,
            (false, false),
        )?;
        // SAFETY: the renderer owns these GL objects; its EGL context is current throughout the operation.
        unsafe {
            let gl = &self.gl;
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture.id),
                0,
            );
            draw(self);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.delete_framebuffer(fbo);
        }
        Ok(texture)
    }

    pub(crate) fn overlay(&self, texture: &Texture, width: u32, height: u32, opacity: f32) {
        // SAFETY: the renderer owns these GL objects; its EGL context is current throughout the operation.
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(texture.id));
        }
        let mut quad = Quad::new((0.0, 0.0), (width as f32, height as f32), 6, Blend::Normal);
        quad.uv = [0.0, 1.0, 1.0, -1.0];
        quad.opacity = opacity;
        self.emit_quad(&quad, width as f32, height as f32);
    }

    fn emit(&self, q: &Quad, w: f32, h: f32) -> Bounds {
        let bounds = quad_bounds(q);
        self.emit_quad(q, w, h);
        bounds
    }

    /// Puts the ring program (`ring`) or the main one in use.
    fn bind(&self, ring: bool) {
        let ring = ring && self.ring.is_some();
        if self.ring_bound.replace(ring) != ring {
            let program = match &self.ring {
                Some((program, _)) if ring => *program,
                _ => self.program,
            };
            // SAFETY: the renderer owns these GL objects; its EGL context is current throughout the operation.
            unsafe { self.gl.use_program(Some(program)) };
        }
    }

    fn emit_quad(&self, q: &Quad, w: f32, h: f32) {
        self.bind(q.kind == 4);
        let loc = match &self.ring {
            Some((_, loc)) if self.ring_bound.get() => loc,
            _ => &self.loc,
        };
        let (sw, sh) = (q.size.0 * q.scale, q.size.1 * q.scale);
        let pivot = (
            q.origin.0 + q.size.0 * q.pivot.0,
            q.origin.1 + q.size.1 * q.pivot.1,
        );
        let corner = (
            pivot.0 + (q.origin.0 - pivot.0) * q.scale,
            pivot.1 + (q.origin.1 - pivot.1) * q.scale,
        );
        let (sin, cos) = q.rotation.to_radians().sin_cos();
        let rel = (corner.0 - pivot.0, corner.1 - pivot.1);
        let tx = pivot.0 + cos * rel.0 - sin * rel.1;
        let ty = pivot.1 + sin * rel.0 + cos * rel.1;
        let a = [cos * sw, sin * sw, -sin * sh, cos * sh];
        let (kx, ky) = (2.0 / w, -2.0 / h);
        let xform = [
            a[0] * kx,
            a[1] * ky,
            0.0,
            a[2] * kx,
            a[3] * ky,
            0.0,
            tx * kx - 1.0,
            ty * ky + 1.0,
            1.0,
        ];
        // SAFETY: the renderer owns these GL objects; its EGL context is current throughout the operation.
        unsafe {
            let gl = &self.gl;
            gl.uniform_matrix_3_f32_slice(loc.xform.as_ref(), false, &xform);
            gl.uniform_1_i32(loc.kind.as_ref(), q.kind);
            gl.uniform_4_f32_slice(loc.color.as_ref(), &q.color);
            gl.uniform_4_f32_slice(loc.color2.as_ref(), &q.color2);
            gl.uniform_4_f32_slice(loc.params.as_ref(), &q.params);
            gl.uniform_4_f32_slice(loc.rect.as_ref(), &q.rect);
            gl.uniform_4_f32_slice(loc.uv.as_ref(), &q.uv);
            gl.uniform_1_f32(loc.opacity.as_ref(), q.opacity);
            gl.uniform_1_f32(
                loc.aspect.as_ref(),
                if q.size.1 > 0.0 {
                    q.size.0 / q.size.1
                } else {
                    1.0
                },
            );
            if q.kind == 4 || q.kind == 5 || q.kind == 9 {
                gl.uniform_1_i32(loc.count.as_ref(), q.count);
            }
            match q.blend {
                Blend::Normal => gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA),
                Blend::Add => gl.blend_func(glow::ONE, glow::ONE),
                Blend::Multiply => gl.blend_func(glow::DST_COLOR, glow::ONE_MINUS_SRC_ALPHA),
            }
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Drawing combines caller-owned scene, layer and framebuffer state."
    )]
    fn particles(
        &self,
        p: &Particles,
        props: &Props,
        shift: f32,
        time: f64,
        w: f32,
        h: f32,
        sprite: Option<&Texture>,
        blend: Blend,
    ) -> Area {
        let mut area = None;
        let [left, top, right, bottom] = p.area;
        // A shared minimum margin keeps equal-seed layers (a glow and its core)
        // on the same paths.
        let margin_y = (p.size.1 * 2.0 * p.stretch.max(1.0)).max(0.04);
        let margin_x = margin_y * h / w;
        let (span_x, span_y) = (right - left + margin_x * 2.0, bottom - top + margin_y * 2.0);
        if let Some(texture) = sprite {
            // SAFETY: the borrowed texture belongs to this renderer and its EGL context is current.
            unsafe { self.gl.bind_texture(glow::TEXTURE_2D, Some(texture.id)) };
        }
        for i in 0..p.count {
            let r = |salt| hash(i, p.seed, salt);
            let speed = 1.0 + p.jitter * (r(3) * 2.0 - 1.0);
            let t = time as f32;
            let wobble = p.wobble
                * (std::f32::consts::TAU * (t / p.wobble_period * (0.7 + 0.6 * r(4)) + r(5))).sin();
            let x = (r(1) * span_x + p.velocity.0 * speed * t + wobble * (h / w))
                .rem_euclid(span_x)
                + left
                - margin_x;
            let y = (r(2) * span_y + p.velocity.1 * speed * t).rem_euclid(span_y) + top - margin_y;
            let radius = p.size.0 + (p.size.1 - p.size.0) * r(6) * r(6);
            let twinkle = 1.0
                - p.twinkle
                    * (0.5 + 0.5 * (std::f32::consts::TAU * (t / (1.5 + 3.5 * r(7)) + r(8))).sin());
            let color = p.colors[((r(9) * p.colors.len() as f32) as usize).min(p.colors.len() - 1)];
            let d = 2.0 * radius * h * props.scale;
            let centre = (
                (x + props.x - 0.5 + shift * (0.7 + 0.6 * r(10))) * w,
                (y + props.y - 0.5) * h,
            );
            let mut quad = if sprite.is_some() {
                let mut quad = Quad::centred(centre, (d, d), 6, blend);
                quad.rotation = p.spin * t * (r(11) * 2.0 - 1.0) + r(12) * 360.0;
                quad
            } else {
                let mut quad = Quad::centred(centre, (d * p.stretch, d), 3, blend);
                quad.color = rgba(color);
                quad.params[0] = p.softness.max(0.002);
                quad.rotation = props.rotation;
                quad
            };
            quad.opacity = props.opacity * twinkle;
            grow(&mut area, self.emit(&quad, w, h));
        }
        area
    }

    /// Draws one frame and returns the area each layer painted, so callers
    /// can tell the compositor which pixels changed.
    pub(crate) fn draw(
        &self,
        scene: &Scene,
        state: &Evaluated,
        width: u32,
        height: u32,
        res: &Resources,
    ) -> Vec<Area> {
        let (w, h) = (width as f32, height as f32);
        let gl = &self.gl;
        let loc = &self.loc;
        let mut areas: Vec<Area> = vec![None; scene.layers.len()];
        // SAFETY: the renderer owns these GL objects; its EGL context is current throughout the operation.
        unsafe {
            gl.viewport(0, 0, width as i32, height as i32);
            let bg = scene.background;
            gl.clear_color(bg.r, bg.g, bg.b, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(self.program));
            self.ring_bound.set(false);
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);
        }
        for (index, (layer, st)) in scene.layers.iter().zip(&state.layers).enumerate() {
            let p = st.props;
            if p.opacity <= 0.0 || p.scale <= 0.0 {
                continue;
            }
            let centre = ((p.x + st.shift) * w, p.y * h);
            let box_size = (p.w * w, p.h * h);
            let finish = |mut quad: Quad| {
                quad.scale = p.scale;
                quad.rotation = p.rotation;
                quad.pivot = layer.pivot;
                if quad.opacity == 1.0 {
                    quad.opacity = p.opacity;
                }
                quad
            };
            match &layer.kind {
                Kind::Solid { color, corner } => {
                    let mut quad = Quad::centred(centre, box_size, 0, layer.blend);
                    quad.color = rgba(*color);
                    quad.params = [
                        corner * h * p.scale,
                        box_size.0 * p.scale,
                        box_size.1 * p.scale,
                        0.0,
                    ];
                    grow(&mut areas[index], self.emit(&finish(quad), w, h));
                }
                Kind::Gradient {
                    stops,
                    angle,
                    radial,
                } => {
                    let flat: Vec<f32> = stops
                        .iter()
                        .flat_map(|c| rgba(*c))
                        .chain(std::iter::repeat(0.0))
                        .take(16)
                        .collect();
                    self.bind(false);
                    // SAFETY: the renderer owns these GL objects; its EGL context is current throughout the operation.
                    unsafe {
                        gl.uniform_4_f32_slice(loc.stops.as_ref(), &flat);
                        gl.uniform_1_i32(loc.count.as_ref(), stops.len() as i32);
                    }
                    let mut quad =
                        Quad::centred(centre, box_size, if *radial { 2 } else { 1 }, layer.blend);
                    quad.params[0] = angle.to_radians();
                    grow(&mut areas[index], self.emit(&finish(quad), w, h));
                }
                Kind::Orb {
                    color,
                    softness,
                    stretch,
                } => {
                    let d = 2.0 * p.radius * h;
                    let mut quad = Quad::centred(centre, (d * stretch, d), 3, layer.blend);
                    quad.color = rgba(*color);
                    quad.params[0] = softness.max(0.002);
                    grow(&mut areas[index], self.emit(&finish(quad), w, h));
                }
                Kind::Ring {
                    color,
                    thickness,
                    softness,
                    sweep,
                    start,
                    track,
                    flat,
                    ..
                } => {
                    let d = 2.0 * p.radius * h;
                    if let Some(track) = track {
                        let mut quad = Quad::centred(centre, (d, d), 4, layer.blend);
                        quad.color = rgba(*track);
                        quad.params = [*thickness, *softness, 1.0, 0.0];
                        grow(&mut areas[index], self.emit(&finish(quad), w, h));
                    }
                    let mut quad = Quad::centred(centre, (d, d), 4, layer.blend);
                    quad.color = rgba(*color);
                    quad.count = i32::from(*flat);
                    quad.params = [
                        *thickness,
                        *softness,
                        res.gauges[index].unwrap_or(*sweep),
                        *start,
                    ];
                    grow(&mut areas[index], self.emit(&finish(quad), w, h));
                }
                Kind::Wave {
                    color,
                    shade,
                    amplitude,
                    frequency,
                    detail,
                    softness,
                    fill,
                } => {
                    let reach = amplitude + softness * 2.0;
                    let (top, mut bottom) = match fill {
                        WaveFill::Down => ((p.y - reach).max(0.0), 1.0),
                        WaveFill::Up => (0.0, (p.y + reach).min(1.0)),
                        WaveFill::Band => ((p.y - reach).max(0.0), (p.y + reach).min(1.0)),
                        WaveFill::Rise => (
                            (p.y - reach).max(0.0),
                            (p.y + amplitude + softness * 0.2).min(1.0),
                        ),
                    };
                    let mode = match fill {
                        WaveFill::Down => 0.0,
                        WaveFill::Up => 1.0,
                        WaveFill::Band => 2.0,
                        WaveFill::Rise => 3.0,
                    };
                    let fill_bottom = bottom;
                    if *fill == WaveFill::Down {
                        bottom = bottom.min(hidden_below(scene, state, index)).max(top);
                    }
                    let mut quad = Quad::new(
                        (0.0, top * h),
                        (w, (bottom - top).max(0.0) * h),
                        5,
                        layer.blend,
                    );
                    quad.color = [color.r, color.g, color.b, 1.0];
                    quad.color2 = shade.map_or([0.0; 4], |s| [s.r, s.g, s.b, fill_bottom]);
                    quad.count = if shade.is_some() { -1 } else { 0 };
                    quad.params = [*amplitude, *frequency, *detail, st.shift];
                    quad.rect = [0.0, top, 1.0, (bottom - top).max(0.0)];
                    quad.uv = [p.phase.rem_euclid(1.0), p.y, *softness, mode];
                    quad.opacity = p.opacity * color.a;
                    grow(&mut areas[index], self.emit(&quad, w, h));
                }
                Kind::Image {
                    fit, tile, repeat, ..
                } => {
                    let Some(texture) = &res.images[index] else {
                        continue;
                    };
                    let (iw, ih) = (texture.width as f32, texture.height as f32);
                    let (mut bw, mut bh) = box_size;
                    let mut uv = [0.0, 0.0, 1.0, 1.0];
                    match fit {
                        Fit::Stretch => {}
                        Fit::Cover => {
                            let s = (bw / iw).max(bh / ih);
                            uv[2] = bw / (iw * s);
                            uv[3] = bh / (ih * s);
                            uv[0] = (1.0 - uv[2]) / 2.0;
                            uv[1] = (1.0 - uv[3]) / 2.0;
                        }
                        Fit::Contain => {
                            let s = (bw / iw).min(bh / ih);
                            (bw, bh) = (iw * s, ih * s);
                        }
                    }
                    uv[2] *= repeat.0;
                    uv[3] *= repeat.1;
                    // SAFETY: the borrowed texture belongs to this renderer and its EGL context is current.
                    unsafe { gl.bind_texture(glow::TEXTURE_2D, Some(texture.id)) };
                    let mut quad = if *tile {
                        uv[0] += p.phase.rem_euclid(1.0) - st.shift * w / bw * uv[2];
                        Quad::centred((p.x * w, centre.1), (bw, bh), 6, layer.blend)
                    } else {
                        Quad::centred(centre, (bw, bh), 6, layer.blend)
                    };
                    quad.uv = uv;
                    grow(&mut areas[index], self.emit(&finish(quad), w, h));
                }
                Kind::Text(text) => {
                    let Some(tex) = &res.texts[index] else {
                        continue;
                    };
                    let (tw, th) = (tex.fill.width as f32, tex.fill.height as f32);
                    let left = match text.align {
                        Align::Left => centre.0 - tex.pad,
                        Align::Center => centre.0 - tw / 2.0,
                        Align::Right => centre.0 - tw + tex.pad,
                    };
                    // Still text snaps to whole pixels for crispness; moving
                    // text keeps its sub-pixel position so it glides.
                    let moving = state.transitioning
                        || layer.motions.iter().any(|m| {
                            m.trigger == enzwall_scene::Trigger::Loop
                                && matches!(
                                    m.property,
                                    Property::X
                                        | Property::Y
                                        | Property::Scale
                                        | Property::Rotation
                                )
                        });
                    let (ox, oy) = (left, centre.1 - th / 2.0);
                    let origin = if moving {
                        (ox, oy)
                    } else {
                        (ox.round(), oy.round())
                    };
                    let em = text.size * h;
                    if let (Some(shadow), Some(texture)) = (&text.shadow, &tex.shadow) {
                        // SAFETY: the borrowed texture belongs to this renderer and its EGL context is current.
                        unsafe { gl.bind_texture(glow::TEXTURE_2D, Some(texture.id)) };
                        let mut quad = Quad::new(
                            (origin.0 + shadow.x * em, origin.1 + shadow.y * em),
                            (tw, th),
                            7,
                            Blend::Normal,
                        );
                        quad.color = rgba(shadow.color);
                        grow(&mut areas[index], self.emit(&finish(quad), w, h));
                    }
                    if let (Some((color, _)), Some(texture)) = (&text.outline, &tex.outline) {
                        // SAFETY: the borrowed texture belongs to this renderer and its EGL context is current.
                        unsafe { gl.bind_texture(glow::TEXTURE_2D, Some(texture.id)) };
                        let mut quad = Quad::new(origin, (tw, th), 7, layer.blend);
                        quad.color = rgba(*color);
                        grow(&mut areas[index], self.emit(&finish(quad), w, h));
                    }
                    // SAFETY: the borrowed texture belongs to this renderer and its EGL context is current.
                    unsafe { gl.bind_texture(glow::TEXTURE_2D, Some(tex.fill.id)) };
                    let mut quad = Quad::new(origin, (tw, th), 7, layer.blend);
                    quad.color = rgba(text.color);
                    grow(&mut areas[index], self.emit(&finish(quad), w, h));
                }
                Kind::Particles(particles) => {
                    let sprite = particles.sprite.as_ref().and(res.images[index].as_ref());
                    areas[index] = self.particles(
                        particles,
                        &p,
                        st.shift,
                        state.loop_time,
                        w,
                        h,
                        sprite,
                        layer.blend,
                    );
                }
                Kind::Grid {
                    color,
                    density,
                    columns,
                    thickness,
                    fade,
                } => {
                    let mut quad = Quad::centred((p.x * w, centre.1), box_size, 8, layer.blend);
                    quad.color = rgba(*color);
                    quad.params = [*density, *columns, *thickness, *fade];
                    quad.uv[0] = p.phase.rem_euclid(1.0);
                    quad.uv[1] = st.shift * w / box_size.0.max(1.0) * 3.0;
                    grow(&mut areas[index], self.emit(&finish(quad), w, h));
                }
                Kind::Graph {
                    color,
                    fill,
                    thickness,
                    glow,
                    ..
                } => {
                    let Some(data) = &res.graphs[index] else {
                        continue;
                    };
                    if data.samples.len() < 2 {
                        continue;
                    }
                    let mut samples = [0.0f32; 64];
                    for (slot, v) in samples.iter_mut().zip(&data.samples) {
                        *slot = *v;
                    }
                    self.bind(false);
                    // SAFETY: the live program owns this uniform and samples contains the required 64 floats.
                    unsafe { gl.uniform_1_f32_slice(loc.samples.as_ref(), &samples) };
                    let mut quad = Quad::centred(centre, box_size, 9, layer.blend);
                    quad.color = rgba(*color);
                    quad.color2 = fill.map_or([0.0; 4], rgba);
                    quad.count = data.samples.len() as i32;
                    quad.params = [*thickness, *glow, 0.0, data.scroll];
                    grow(&mut areas[index], self.emit(&finish(quad), w, h));
                }
            }
        }
        if state.dim > 0.0 {
            let mut quad = Quad::new((0.0, 0.0), (w, h), 0, Blend::Normal);
            quad.color = [0.0, 0.0, 0.0, 1.0];
            quad.opacity = state.dim;
            self.emit_quad(&quad, w, h);
        }
        areas
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enzwall_scene::{Runtime, evaluate};

    fn waves(layers: &str) -> (Scene, Evaluated) {
        let scene = Scene::parse(&format!(
            "format = 1\nname = \"T\"\nfps = 24\nbackground = \"#000000\"\n{layers}"
        ))
        .expect("scene parses");
        let state = evaluate(&scene, &Runtime::new(0.0), 0.0, 16.0 / 9.0);
        (scene, state)
    }

    /// Sabotage: also count a translucent or additive wave as a cover. The
    /// wave behind it is then cut off where it still shows.
    #[test]
    fn a_wave_is_cut_off_only_below_an_opaque_wave_in_front() {
        let (scene, state) = waves(
            "[[layer]]\nkind = \"wave\"\ny = 0.4\namplitude = 0.05\nsoftness = 0.01\ncolor = \"#ff0000\"\n\
             [[layer]]\nkind = \"wave\"\ny = 0.6\namplitude = 0.08\nsoftness = 0.02\ncolor = \"#00ff00\"\n\
             [[layer]]\nkind = \"wave\"\nfill = \"band\"\ny = 0.5\ncolor = \"#0000ff\"\n\
             [[layer]]\nkind = \"wave\"\ny = 0.3\namplitude = 0.05\ncolor = \"#ffffff80\"\n\
             [[layer]]\nkind = \"wave\"\ny = 0.2\namplitude = 0.05\ncolor = \"#ffffff\"\nblend = \"add\"\n",
        );
        assert!((hidden_below(&scene, &state, 0) - 0.70).abs() < 1e-5);
        assert_eq!(hidden_below(&scene, &state, 1), 1.0);
    }
}
