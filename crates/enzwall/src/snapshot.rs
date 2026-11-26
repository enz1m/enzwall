#![allow(
    unsafe_code,
    reason = "Offscreen framebuffer operations use the current owned EGL context."
)]
use crate::egl::Context;
use crate::gl::Renderer;
use crate::stage::Stage;
use crate::vars::Vars;
use enzwall_scene::Trigger;
use glow::HasContext;
use std::path::Path;
use std::time::Instant;

pub(crate) enum Cue {
    Event(Trigger, f64),
    Workspace(u32, f64),
}

pub(crate) struct Offscreen {
    _context: Context,
    pub renderer: Renderer,
    width: u32,
    height: u32,
}

impl Offscreen {
    pub(crate) fn new(width: u32, height: u32) -> Result<Self, String> {
        let context = Context::surfaceless()?;
        context.make_current(None)?;
        let renderer = Renderer::new(context.glow())?;
        // SAFETY: the owned surfaceless EGL context is current and all framebuffer objects are created here.
        unsafe {
            let gl = &renderer.gl;
            let fbo = gl.create_framebuffer()?;
            let rbo = gl.create_renderbuffer()?;
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rbo));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, width as i32, height as i32);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::RENDERBUFFER,
                Some(rbo),
            );
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                return Err("offscreen framebuffer incomplete".into());
            }
        }
        Ok(Self {
            _context: context,
            renderer,
            width,
            height,
        })
    }

    fn read(&self) -> Vec<u8> {
        let (w, h) = (self.width as usize, self.height as usize);
        let mut pixels = vec![0u8; w * h * 4];
        // SAFETY: the owned framebuffer is current and pixels has exactly w*h*4 writable bytes.
        unsafe {
            self.renderer.gl.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
        }
        let mut flipped = Vec::with_capacity(pixels.len());
        for row in pixels.chunks_exact(w * 4).rev() {
            flipped.extend(row.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2], 255]));
        }
        flipped
    }
}

pub(crate) fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())
}

/// Which text layers a snapshot keeps, matching the wallpaper's own settings.
#[derive(Clone, Copy, Default)]
pub(crate) struct Shown {
    pub hide_clock: bool,
    pub hide_date: bool,
}

fn prepare(offscreen: &Offscreen, dir: &Path, cues: &[Cue], shown: Shown) -> Result<Stage, String> {
    let mut stage = Stage::load_filtered(
        &offscreen.renderer,
        dir,
        0.0,
        shown.hide_clock,
        shown.hide_date,
    )?;
    for cue in cues {
        match *cue {
            Cue::Event(trigger, at) => stage.runtime.fire(trigger, at),
            Cue::Workspace(index, at) => stage.runtime.workspace(&stage.scene, index, at),
        }
    }
    Ok(stage)
}

/// Renders `frames` images starting at `time`, `interval` seconds apart.
/// With more than one frame, `out` gains a `-NNN` suffix per frame.
#[allow(
    clippy::too_many_arguments,
    reason = "The snapshot CLI supplies independent scene, output, frame sequencing and event options."
)]
pub(crate) fn snapshot(
    dir: &Path,
    out: &Path,
    size: (u32, u32),
    time: f64,
    frames: u32,
    interval: f64,
    cues: &[Cue],
    clock: Option<i64>,
    shown: Shown,
) -> Result<(), String> {
    let offscreen = Offscreen::new(size.0, size.1)?;
    let mut stage = prepare(&offscreen, dir, cues, shown)?;
    let mut vars = Vars::new();
    if let Some(at) = clock {
        vars.freeze_time(at);
    }
    for step in (0..=64).rev() {
        let at = time - f64::from(step) * 2.0;
        vars.set_demo(at);
        stage.refresh_texts(&offscreen.renderer, &mut vars, size.1, at)?;
    }
    for frame in 0..frames {
        let at = time + f64::from(frame) * interval;
        vars.set_demo(at);
        stage.refresh_texts(&offscreen.renderer, &mut vars, size.1, at)?;
        stage.draw(&offscreen.renderer, at, size.0, size.1);
        let path = if frames > 1 {
            let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("frame");
            out.with_file_name(format!("{stem}-{frame:03}.png"))
        } else {
            out.to_path_buf()
        };
        write_png(&path, size.0, size.1, &offscreen.read())?;
    }
    Ok(())
}

pub(crate) struct Bench {
    pub cpu_ms: f64,
    pub gpu_ms: Option<f64>,
    pub gpu_worst_ms: Option<f64>,
}

/// Benchmarks frames at 60 fps scene time; GPU timing requires driver timer-query support.
pub(crate) fn bench(dir: &Path, size: (u32, u32), frames: u32) -> Result<Bench, String> {
    let offscreen = Offscreen::new(size.0, size.1)?;
    let mut stage = prepare(&offscreen, dir, &[Cue::Workspace(1, 0.5)], Shown::default())?;
    let mut vars = Vars::new();
    stage.refresh_texts(&offscreen.renderer, &mut vars, size.1, 0.0)?;
    let gl = &offscreen.renderer.gl;
    let timer = gl
        .supported_extensions()
        .iter()
        .any(|e| e.contains("timer_query"));
    let mut cpu = 0.0;
    let mut gpu = Vec::new();
    for frame in 0..frames {
        let query = if timer {
            // SAFETY: the owned EGL context is current and advertises timer queries.
            unsafe { gl.create_query().ok() }
        } else {
            None
        };
        if let Some(q) = query {
            // SAFETY: q is a newly created query and no elapsed-time query is active.
            unsafe { gl.begin_query(glow::TIME_ELAPSED, q) };
        }
        let at = Instant::now();
        stage.draw(&offscreen.renderer, f64::from(frame) / 60.0, size.0, size.1);
        cpu += at.elapsed().as_secs_f64();
        if let Some(q) = query {
            // SAFETY: q owns the active query; the current context remains live through readback and deletion.
            unsafe {
                gl.end_query(glow::TIME_ELAPSED);
                gl.finish();
                gpu.push(f64::from(gl.get_query_parameter_u32(q, glow::QUERY_RESULT)) / 1e6);
                gl.delete_query(q);
            }
        } else {
            // SAFETY: the owned EGL context remains current while waiting for its work.
            unsafe { gl.finish() };
        }
    }
    let mean = (!gpu.is_empty()).then(|| gpu.iter().sum::<f64>() / gpu.len() as f64);
    let worst = gpu
        .iter()
        .copied()
        .fold(None, |w: Option<f64>, v| Some(w.map_or(v, |w| w.max(v))));
    Ok(Bench {
        cpu_ms: cpu * 1000.0 / f64::from(frames),
        gpu_ms: mean,
        gpu_worst_ms: worst,
    })
}

// Binary output is deliberately fixed-size and contains pixels only. A greeter
// never loads a package executable or gives a scene an input/session socket.
pub(crate) fn stream(
    dir: &Path,
    size: (u32, u32),
    fps: u32,
    hide_clock: bool,
    hide_date: bool,
    still: bool,
) -> Result<(), String> {
    use std::io::Write;
    let offscreen = Offscreen::new(size.0, size.1)?;
    let mut stage = Stage::load_filtered(&offscreen.renderer, dir, 0.0, hide_clock, hide_date)?;
    let clock = stage.scene.layers.iter().any(|l| {
        matches!(
            l.role,
            enzwall_scene::LayerRole::Clock | enzwall_scene::LayerRole::ClockDate
        )
    });
    let date = stage.scene.layers.iter().any(|l| {
        matches!(
            l.role,
            enzwall_scene::LayerRole::Date | enzwall_scene::LayerRole::ClockDate
        )
    });
    let mut out = std::io::stdout().lock();
    out.write_all(b"ENZWALL1")
        .and_then(|()| out.write_all(&size.0.to_le_bytes()))
        .and_then(|()| out.write_all(&size.1.to_le_bytes()))
        .and_then(|()| out.write_all(&(u32::from(clock) | u32::from(date) << 1).to_le_bytes()))
        .map_err(|_| "stream closed")?;
    let start = Instant::now();
    let interval = std::time::Duration::from_secs_f64(1.0 / f64::from(fps));
    let mut vars = Vars::new();
    vars.public = true;
    // Runtime readings in a public login scene never expose host/session data.
    for name in ["host", "user", "cpu", "mem", "battery", "load"] {
        vars.set_custom(name.into(), String::new())?;
    }
    loop {
        let frame = Instant::now();
        let now = if still {
            30.0
        } else {
            start.elapsed().as_secs_f64() + 30.0
        };
        stage.refresh_texts(&offscreen.renderer, &mut vars, size.1, now)?;
        stage.draw(&offscreen.renderer, now, size.0, size.1);
        if out
            .write_all(&offscreen.read())
            .and_then(|()| out.flush())
            .is_err()
        {
            return Ok(());
        }
        std::thread::sleep(interval.saturating_sub(frame.elapsed()));
    }
}
