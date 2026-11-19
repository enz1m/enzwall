#![allow(
    unsafe_code,
    reason = "Wayland EGL windows, POSIX polling and signal handlers require native calls."
)]
use crate::egl::Context;
use crate::gl::{Area, Bounds, Renderer};
use crate::hyprland::{HyprEvent, Hyprland};
use crate::ipc::{Command, Server};
use crate::pacing::{self, Smoothness};
use crate::stage::Stage;
use crate::vars::Vars;
use enzwall_scene::Trigger;
use khronos_egl as egl;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
    globals::registry_queue_init,
    protocol::{wl_output, wl_seat, wl_surface},
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1, ext_idle_notifier_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_sys::egl::{wl_egl_window_create, wl_egl_window_destroy, wl_egl_window_resize};

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

pub(crate) struct Options {
    pub scene: PathBuf,
    pub output: Option<String>,
    pub namespace: String,
    pub socket: PathBuf,
    pub hyprland: Option<PathBuf>,
    pub ready: bool,
    pub hide_clock: bool,
    pub hide_date: bool,
    pub idle_timeout: Option<u32>,
    pub smooth: Smoothness,
}

// Use CLOCK_MONOTONIC milliseconds to match compositor frame timestamps.
fn mono_ms() -> f64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: ts is writable timespec storage and CLOCK_MONOTONIC accepts no other pointers.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1e6
}

struct Gpu {
    context: Context,
    renderer: Renderer,
    window: *mut wayland_sys::egl::wl_egl_window,
    surface: egl::Surface,
    size: (u32, u32),
}

struct App {
    registry: RegistryState,
    outputs: OutputState,
    compositor: CompositorState,
    layer: Option<LayerSurface>,
    viewport: Option<wp_viewport::WpViewport>,
    logical: (u32, u32),
    scale: i32,
    configured: bool,
    frame_pending: bool,
    presented: bool,
    closed: bool,
    idle: Option<bool>,
    // Compositor timestamps (ms) of presented frames, for pacing statistics.
    last_present: Option<u32>,
    intervals: std::collections::VecDeque<f32>,
    // Most recent frame-callback vblank on the process monotonic clock (ms).
    vblank: Option<f64>,
    // Smallest seen delay between a callback's timestamp and its arrival.
    callback_delay: Option<f64>,
    refresh_mhz: i32,
    // The pending frame callback belongs to an empty commit used only to
    // follow the compositor's refresh, not to a rendered frame.
    tick_pending: bool,
}

struct Engine {
    role_filter: (bool, bool),
    gpu: Option<Gpu>,
    stage: Option<Stage>,
    vars: Vars,
    start_ms: f64,
    dirty: bool,
    next_at: Option<f64>,
    frames: u64,
    skipped: u64,
    smooth: Smoothness,
    mains: bool,
    power_checked: f64,
    shown: Option<(enzwall_scene::Evaluated, f64)>,
    areas: Vec<Area>,
    loop_fps: u32,
    // The vblank the next frame is meant for (ms).
    planned: Option<f64>,
    // Follow every refresh with empty commits instead of timers; timers
    // alone mis-predict when the compositor's own refresh wanders.
    tick_mode: bool,
    sleeping: bool,
    asleep: bool,
    paused: bool,
    /// A fullscreen window covers the output (Hyprland only).
    covered: bool,
    fps_override: Option<u32>,
    output_name: Option<String>,
    fade: Option<(crate::gl::Texture, f64)>,
}

const CROSSFADE: f64 = 0.8;
const GAUGE_FPS: u32 = 15;

impl Engine {
    fn now(&self) -> f64 {
        (mono_ms() - self.start_ms) / 1000.0
    }

    fn refresh_hz(app: &App) -> f64 {
        (f64::from(app.refresh_mhz) / 1000.0).clamp(20.0, 500.0)
    }

    fn loop_rate(&mut self, app: &App, stage: &Stage) -> u32 {
        if let Some(fps) = self.fps_override {
            return fps;
        }
        let now = mono_ms();
        if now - self.power_checked > 30_000.0 {
            self.power_checked = now;
            self.mains = pacing::on_mains();
        }
        let smooth = match self.smooth {
            Smoothness::Balanced => false,
            Smoothness::Smooth => true,
            Smoothness::Auto => self.mains,
        };
        if smooth {
            Self::refresh_hz(app).round() as u32
        } else {
            stage.scene.fps
        }
    }

    fn status(&self, app: &App) -> serde_json::Value {
        let rss = std::fs::read_to_string("/proc/self/statm")
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .map(|pages| pages * 4);
        let state = self.stage.as_ref().and_then(|s| s.last.as_ref());
        serde_json::json!({
            "ok": true,
            "scene": self.stage.as_ref().map(|s| s.dir.display().to_string()),
            "name": self.stage.as_ref().map(|s| s.scene.name.clone()),
            "frames": self.frames,
            "skipped": self.skipped,
            "smoothness": self.smooth.name(),
            "loop_fps": self.loop_fps,
            "refresh_hz": Self::refresh_hz(app),
            "size": [app.logical.0, app.logical.1],
            "buffer": self.gpu.as_ref().map(|g| [g.size.0, g.size.1]),
            "looping": !self.asleep && state.is_some_and(|s| s.looping),
            "transitioning": state.is_some_and(|s| s.transitioning),
            "asleep": self.asleep,
            "paused": self.paused,
            "covered": self.covered,
            "desktop": self.stage.as_ref().map(|s| s.runtime.desktop()),
            "output": self.output_name,
            "partial_damage": self.gpu.as_ref().is_some_and(|g| g.context.has_partial_damage()),
            "frame_ms": pacing(&app.intervals).map(|(m, _)| m),
            "jitter_ms": pacing(&app.intervals).map(|(_, j)| j),
            "intervals_ms": app.intervals.iter().map(|v| v.round() as i32).collect::<Vec<_>>(),
            "rss_kib": rss,
        })
    }

    fn command(&mut self, app: &App, command: Command) -> serde_json::Value {
        let now = self.now();
        let ok = serde_json::json!({"ok": true});
        match command {
            Command::Status => return self.status(app),
            Command::Event(trigger) => {
                if let Some(stage) = &mut self.stage {
                    match trigger {
                        Trigger::Sleep => {
                            stage.runtime.fire(Trigger::Sleep, now);
                            self.sleeping = true;
                        }
                        Trigger::Wake | Trigger::Unlock => self.wake(trigger),
                        other => stage.runtime.fire(other, now),
                    }
                }
            }
            Command::Workspace(index) => self.workspace(index),
            Command::Set(name, value) => {
                if let Err(error) = self.vars.set_custom(name, value) {
                    return serde_json::json!({"ok":false,"error":error});
                }
            }
            Command::Unset(name) => {
                self.vars.unset_custom(&name);
            }
            Command::Load(dir) => {
                let Some(gpu) = &self.gpu else {
                    return serde_json::json!({"ok": false, "error": "not presented yet"});
                };
                match Stage::load_filtered(
                    &gpu.renderer,
                    &dir,
                    now,
                    self.role_filter.0,
                    self.role_filter.1,
                ) {
                    Ok(mut stage) => {
                        if let Some(mut old) = self.stage.take() {
                            stage.runtime.jump(old.runtime.desktop(), now);
                            let (w, h) = gpu.size;
                            if let Ok(frame) = gpu.renderer.capture(w, h, |r| {
                                old.draw(r, now, w, h);
                            }) && let Some((previous, _)) = self.fade.replace((frame, now))
                            {
                                gpu.renderer.delete(previous);
                            }
                            old.release(&gpu.renderer);
                        }
                        self.shown = None;
                        self.areas.clear();
                        if self.covered {
                            stage.runtime.pause_loops(now);
                        }
                        self.stage = Some(stage);
                    }
                    Err(error) => return serde_json::json!({"ok": false, "error": error}),
                }
            }
            Command::Pause => {
                self.paused = true;
                if let Some(stage) = &mut self.stage {
                    stage.runtime.pause_loops(now);
                }
            }
            Command::Resume => {
                self.paused = false;
                if let Some(stage) = &mut self.stage
                    && !self.asleep
                    && !self.covered
                {
                    stage.runtime.resume_loops(now);
                }
            }
            Command::Fps(fps) => self.fps_override = fps,
            Command::Smooth(smooth) => self.smooth = smooth,
        }
        self.dirty = true;
        ok
    }

    fn wake(&mut self, trigger: Trigger) {
        let now = self.now();
        if let Some(stage) = &mut self.stage {
            if !self.paused && !self.covered {
                stage.runtime.resume_loops(now);
            }
            stage.runtime.fire(trigger, now);
        }
        self.sleeping = false;
        self.asleep = false;
        self.dirty = true;
    }

    /// A fullscreen window hides the wallpaper: its loops stop, so nothing
    /// is drawn until it shows again (a clock still turns, once a minute).
    fn set_covered(&mut self, covered: bool) {
        if self.covered == covered {
            return;
        }
        self.covered = covered;
        let now = self.now();
        if let Some(stage) = &mut self.stage {
            if covered {
                stage.runtime.pause_loops(now);
            } else if !self.paused && !self.asleep {
                stage.runtime.resume_loops(now);
                self.dirty = true;
            }
        }
    }

    fn workspace(&mut self, index: u32) {
        let now = self.now();
        if let Some(stage) = &mut self.stage
            && stage.runtime.desktop() != index
        {
            stage.runtime.workspace(&stage.scene, index, now);
            self.dirty = true;
        }
    }

    fn due(&self) -> bool {
        self.dirty || self.next_at.is_some_and(|at| at <= mono_ms())
    }

    fn render(&mut self, app: &mut App, qh: &QueueHandle<App>) -> Result<(), String> {
        let period = 1000.0 / Self::refresh_hz(app);
        let now_ms = mono_ms();
        // Evaluate motion for the vblank this frame will be shown on, so
        // steps are even however late the process woke.
        let mut target = app.vblank.map_or(now_ms, |v| {
            v + (((now_ms - v) / period).floor().max(0.0) + 1.0) * period
        });
        // Never show a frame earlier than the one already planned: a wake a
        // little before its vblank must not land one refresh early.
        if let Some(planned) = self.planned
            && target < planned - period * 0.5
            && !self.dirty
        {
            target = planned;
        }
        let t = (target - self.start_ms) / 1000.0;
        let Some(stage) = self.stage.take() else {
            return Ok(());
        };
        let mut stage = stage;
        let result = self.render_stage(app, qh, &mut stage, target, t, period);
        self.stage = Some(stage);
        result
    }

    fn render_stage(
        &mut self,
        app: &mut App,
        qh: &QueueHandle<App>,
        stage: &mut Stage,
        target: f64,
        t: f64,
        period: f64,
    ) -> Result<(), String> {
        let (Some(gpu), Some(layer)) = (&mut self.gpu, &app.layer) else {
            return Ok(());
        };
        let (w, h) = (gpu.size.0 as f32, gpu.size.1 as f32);
        let changed = stage.refresh_texts(&gpu.renderer, &mut self.vars, gpu.size.1, t)?;
        let state = stage.evaluate(t, gpu.size.0, gpu.size.1);
        let gauges = stage.gauges_moving();
        let full = self.dirty
            || self.fade.is_some()
            || self.shown.is_none()
            || self.areas.len() != stage.scene.layers.len();
        let deltas = match &self.shown {
            Some((shown, at)) if !full => {
                pacing::changes(&stage.scene, shown, &state, (t - at) as f32, w, h)
            }
            _ => Vec::new(),
        };
        let dim_changed = self
            .shown
            .as_ref()
            .is_some_and(|(s, _)| (s.dim - state.dim).abs() * 255.0 >= pacing::THRESHOLD);
        let moving: Vec<bool> = (0..stage.scene.layers.len())
            .map(|i| {
                deltas.get(i).is_some_and(|d| *d >= pacing::THRESHOLD)
                    || changed.contains(&i)
                    || (gauges
                        && matches!(
                            stage.scene.layers[i].kind,
                            enzwall_scene::Kind::Ring { bind: Some(_), .. }
                        ))
            })
            .collect();
        let visible = full || dim_changed || moving.iter().any(|m| *m);
        if visible {
            let areas = stage.draw_state(&gpu.renderer, state, t, gpu.size.0, gpu.size.1);
            if let Some((frame, started)) = &self.fade {
                let f = ((t - started) / CROSSFADE).clamp(0.0, 1.0) as f32;
                if f < 1.0 {
                    gpu.renderer.overlay(
                        frame,
                        gpu.size.0,
                        gpu.size.1,
                        1.0 - enzwall_scene::Easing::InOutSine.apply(f),
                    );
                } else if let Some((frame, _)) = self.fade.take() {
                    gpu.renderer.delete(frame);
                }
            }
            // Tell the compositor which pixels changed: the old and new areas
            // of every layer that visibly moved.
            let damage = if full || dim_changed {
                None
            } else {
                let rects: Vec<Bounds> = moving
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| **m)
                    .filter_map(|(i, _)| match (self.areas[i], areas[i]) {
                        (Some(a), Some(b)) => Some(a.union(b)),
                        (a, b) => a.or(b),
                    })
                    .collect();
                let area: f32 = rects.iter().map(|r| r.w * r.h).sum();
                (area < 0.5 * w * h).then(|| {
                    rects
                        .iter()
                        .map(|b| {
                            [
                                b.x.floor() as i32 - 2,
                                b.y.floor() as i32 - 2,
                                b.w.ceil() as i32 + 4,
                                b.h.ceil() as i32 + 4,
                            ]
                        })
                        .collect::<Vec<_>>()
                })
            };
            self.areas = areas;
            layer.wl_surface().frame(qh, layer.wl_surface().clone());
            gpu.context
                .swap(gpu.surface, damage.as_deref(), gpu.size.1 as i32)?;
            app.frame_pending = true;
            self.frames += 1;
            self.shown = stage.last.clone().map(|s| (s, t));
        } else {
            self.skipped += 1;
        }
        self.dirty = false;

        let (transitioning, looping) = self
            .shown
            .as_ref()
            .map_or((false, false), |(s, _)| (s.transitioning, s.looping));
        if self.sleeping && !transitioning {
            stage.runtime.pause_loops(t);
            self.asleep = true;
        }
        let refresh = Self::refresh_hz(app);
        let loop_rate = self.loop_rate(app, stage);
        self.loop_fps = if looping && !self.asleep {
            loop_rate
        } else {
            0
        };
        let rate = if transitioning || self.fade.is_some() {
            stage.scene.transition_fps.max(loop_rate)
        } else if gauges && !self.asleep {
            GAUGE_FPS.max(self.loop_fps)
        } else {
            self.loop_fps
        };
        // Whole vblanks per frame, so every frame is shown for the same time.
        let divisor = (refresh / f64::from(rate.max(1))).round().max(1.0);
        self.planned = (rate > 0).then_some(target + divisor * period);
        // Slower rates wake before their target vblank to avoid being presented one refresh
        // early.
        let lead = if divisor > 1.0 && !self.tick_mode {
            0.7
        } else {
            1.0
        };
        let motion_at = self.planned.map(|p| p - period * lead);
        let text_at = if self.asleep {
            None
        } else {
            stage
                .next_text_change(self.vars.now_ms())
                .map(|d| mono_ms() + d.as_secs_f64() * 1000.0)
        };
        self.next_at = match (motion_at, text_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        Ok(())
    }

    fn resize(&mut self, app: &App, context_display: *mut std::ffi::c_void) -> Result<(), String> {
        let Some(layer) = &app.layer else {
            return Ok(());
        };
        let render_scale = self.stage.as_ref().map_or(1.0, |s| s.scene.render_scale);
        let physical =
            |v: u32| ((v as f32 * app.scale as f32 * render_scale).round() as u32).max(1);
        let size = (physical(app.logical.0), physical(app.logical.1));
        match &mut self.gpu {
            Some(gpu) if gpu.size == size => {}
            Some(gpu) => {
                // SAFETY: gpu owns a live wl_egl_window and dimensions are bounded by scene/output limits.
                unsafe {
                    wayland_sys::ffi_dispatch!(
                        wayland_egl_handle,
                        wl_egl_window_resize,
                        gpu.window,
                        size.0 as i32,
                        size.1 as i32,
                        0,
                        0
                    );
                }
                gpu.size = size;
            }
            None => {
                let context = Context::wayland(context_display)?;
                let surface_ptr = layer.wl_surface().id().as_ptr();
                // SAFETY: layer retains the live wl_surface until the EGL window is destroyed.
                let window = unsafe {
                    wayland_sys::ffi_dispatch!(
                        wayland_egl_handle,
                        wl_egl_window_create,
                        surface_ptr.cast(),
                        size.0 as i32,
                        size.1 as i32
                    )
                };
                if window.is_null() {
                    return Err("wl_egl_window_create failed".into());
                }
                // SAFETY: window is non-null and the matching Wayland EGL display and config remain live.
                let surface = unsafe {
                    context
                        .egl
                        .create_window_surface(
                            context.display,
                            context.config,
                            window as egl::NativeWindowType,
                            None,
                        )
                        .map_err(|e| format!("EGL window surface: {e}"))?
                };
                context.make_current(Some(surface))?;
                let _ = context.egl.swap_interval(context.display, 0);
                let renderer = Renderer::new(context.glow())?;
                self.gpu = Some(Gpu {
                    context,
                    renderer,
                    window,
                    surface,
                    size,
                });
            }
        }
        if let Some(viewport) = &app.viewport {
            viewport.set_destination(app.logical.0 as i32, app.logical.1 as i32);
        } else {
            layer.wl_surface().set_buffer_scale(app.scale);
        }
        if let Ok(region) = Region::new(&app.compositor) {
            region.add(0, 0, app.logical.0 as i32, app.logical.1 as i32);
            layer
                .wl_surface()
                .set_opaque_region(Some(region.wl_region()));
        }
        self.dirty = true;
        Ok(())
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if factor != self.scale {
            self.scale = factor.clamp(1, 4);
            self.configured = false;
        }
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        time: u32,
    ) {
        self.frame_pending = false;
        self.presented = true;
        let tick = std::mem::take(&mut self.tick_pending);
        if let (false, Some(last)) = (tick, self.last_present) {
            let interval = time.wrapping_sub(last) as f32;
            if interval < 1000.0 {
                self.intervals.push_back(interval);
                if self.intervals.len() > 240 {
                    self.intervals.pop_front();
                }
            } else {
                self.intervals.clear();
            }
        }
        if !tick {
            self.last_present = Some(time);
        }
        let arrived = mono_ms();
        let delay = arrived - f64::from(time);
        // Compositor timestamps are 32-bit milliseconds; only trust them while
        // they agree with the monotonic clock; otherwise use arrival time.
        let delay = if delay.abs() < 5_000.0 {
            self.callback_delay.map_or(delay, |d| d.min(delay))
        } else {
            0.0
        };
        self.callback_delay = Some(delay);
        self.vblank = Some(if delay == 0.0 {
            arrived
        } else {
            f64::from(time) + delay
        });
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.closed = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (w, h) = configure.new_size;
        if w > 0 && h > 0 && (w, h) != self.logical {
            self.logical = (w, h);
        }
        self.configured = false;
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => state.idle = Some(true),
            ext_idle_notification_v1::Event::Resumed => state.idle = Some(false),
            _ => {}
        }
    }
}

delegate_compositor!(App);
delegate_output!(App);
delegate_layer!(App);
delegate_registry!(App);
delegate_noop!(App: ignore wl_seat::WlSeat);
delegate_noop!(App: ext_idle_notifier_v1::ExtIdleNotifierV1);
delegate_noop!(App: wp_viewporter::WpViewporter);
delegate_noop!(App: wp_viewport::WpViewport);

fn pacing(intervals: &std::collections::VecDeque<f32>) -> Option<(f32, f32)> {
    if intervals.len() < 8 {
        return None;
    }
    let n = intervals.len() as f32;
    let mean = intervals.iter().sum::<f32>() / n;
    let var = intervals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n;
    Some((
        (mean * 10.0).round() / 10.0,
        (var.sqrt() * 10.0).round() / 10.0,
    ))
}

fn poll(fds: &mut [libc::pollfd], timeout: Option<Duration>) {
    // Microsecond timeouts: a wake that lands early finds nothing due and has
    // to sleep again, one that lands late shows the frame a refresh too late.
    let ts = timeout.map(|d| libc::timespec {
        tv_sec: d.as_secs() as libc::time_t,
        tv_nsec: libc::c_long::from(d.subsec_nanos()),
    });
    // SAFETY: fds and optional timespec remain live for this call; NULL keeps the signal mask unchanged.
    unsafe {
        libc::ppoll(
            fds.as_mut_ptr(),
            fds.len() as libc::nfds_t,
            ts.as_ref().map_or(std::ptr::null(), |t| t as *const _),
            std::ptr::null(),
        );
    }
}

pub(crate) fn run(options: Options) -> Result<(), String> {
    // SAFETY: the static handler only stores an AtomicBool and has the POSIX signal-handler ABI.
    unsafe { libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t) };
    // SAFETY: SIGINT uses the same static, async-signal-safe handler.
    unsafe { libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t) };
    // SAFETY: SIG_IGN is the platform-defined sentinel for ignoring SIGPIPE.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    let connection = Connection::connect_to_env().map_err(|e| format!("Wayland: {e}"))?;
    let (globals, mut queue) =
        registry_queue_init::<App>(&connection).map_err(|e| format!("Wayland registry: {e}"))?;
    let qh = queue.handle();
    let compositor =
        CompositorState::bind(&globals, &qh).map_err(|e| format!("wl_compositor: {e}"))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|_| "The compositor does not support wlr-layer-shell")?;
    let viewporter: Option<wp_viewporter::WpViewporter> = globals.bind(&qh, 1..=1, ()).ok();
    let mut app = App {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        compositor,
        layer: None,
        viewport: None,
        logical: (0, 0),
        scale: 1,
        configured: false,
        frame_pending: false,
        presented: false,
        closed: false,
        idle: None,
        last_present: None,
        intervals: std::collections::VecDeque::new(),
        vblank: None,
        callback_delay: None,
        refresh_mhz: 60_000,
        tick_pending: false,
    };
    queue
        .roundtrip(&mut app)
        .map_err(|e| format!("Wayland: {e}"))?;
    let (output, output_name) = {
        let mut chosen = None;
        for output in app.outputs.outputs() {
            let info = app.outputs.info(&output);
            let name = info.as_ref().and_then(|i| i.name.clone());
            if options.output.is_none() || name == options.output {
                if let Some(mode) = info
                    .as_ref()
                    .and_then(|i| i.modes.iter().find(|m| m.current))
                    && mode.refresh_rate > 0
                {
                    app.refresh_mhz = mode.refresh_rate;
                }
                app.scale = info.map_or(1, |i| i.scale_factor.clamp(1, 4));
                chosen = Some((output, name));
                break;
            }
        }
        chosen.ok_or_else(|| format!("output {:?} not found", options.output))?
    };
    let surface = app.compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Background,
        Some(options.namespace.clone()),
        Some(&output),
    );
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_size(0, 0);
    if let Ok(region) = Region::new(&app.compositor) {
        layer
            .wl_surface()
            .set_input_region(Some(region.wl_region()));
    }
    app.viewport = viewporter
        .as_ref()
        .map(|v| v.get_viewport(layer.wl_surface(), &qh, ()));
    layer.commit();
    app.layer = Some(layer);

    let seat: Option<wl_seat::WlSeat> = globals.bind(&qh, 1..=1, ()).ok();
    let notifier: Option<ext_idle_notifier_v1::ExtIdleNotifierV1> =
        globals.bind(&qh, 1..=1, ()).ok();

    let mut server = Server::bind(&options.socket)?;
    let mut hyprland = options
        .hyprland
        .as_deref()
        .map(Hyprland::connect)
        .transpose()?;
    let mut engine = Engine {
        role_filter: (options.hide_clock, options.hide_date),
        gpu: None,
        stage: None,
        vars: Vars::new(),
        start_ms: mono_ms(),
        dirty: true,
        next_at: None,
        frames: 0,
        skipped: 0,
        smooth: options.smooth,
        mains: pacing::on_mains(),
        power_checked: mono_ms(),
        shown: None,
        areas: Vec::new(),
        loop_fps: 0,
        planned: None,
        tick_mode: std::env::var_os("ENZWALL_TIMER_PACING").is_none(),
        sleeping: false,
        asleep: false,
        paused: false,
        covered: false,
        fps_override: None,
        output_name: output_name.clone(),
        fade: None,
    };
    let display = connection.backend().display_ptr().cast();
    let mut announced = false;
    let mut idle_notification = None;
    let mut initial_workspace = hyprland
        .as_mut()
        .and_then(|h| h.active_workspace(output_name.as_deref()));

    while !STOP.load(Ordering::SeqCst) && !app.closed {
        queue
            .dispatch_pending(&mut app)
            .map_err(|e| format!("Wayland: {e}"))?;
        if !app.configured && app.logical.0 > 0 {
            engine.resize(&app, display)?;
            app.configured = true;
            if engine.stage.is_none() {
                let gpu = engine
                    .gpu
                    .as_ref()
                    .ok_or("Wallpaper renderer was not configured")?;
                let mut stage = Stage::load_filtered(
                    &gpu.renderer,
                    &options.scene,
                    engine.now(),
                    options.hide_clock,
                    options.hide_date,
                )?;
                if let Some(index) = initial_workspace.take() {
                    stage.runtime.jump(index, engine.now());
                }
                let timeout = options.idle_timeout.unwrap_or(stage.scene.idle_timeout);
                if let (Some(notifier), Some(seat), true) = (&notifier, &seat, timeout > 0) {
                    idle_notification = Some(notifier.get_idle_notification(
                        timeout.saturating_mul(1000),
                        seat,
                        &qh,
                        (),
                    ));
                }
                engine.stage = Some(stage);
                engine.resize(&app, display)?;
                if let Some(covered) = hyprland.as_ref().and_then(Hyprland::covered) {
                    engine.set_covered(covered);
                }
            }
        }
        if let Some(idle) = app.idle.take() {
            if idle {
                engine.command(&app, Command::Event(Trigger::Sleep));
            } else {
                engine.wake(Trigger::Wake);
            }
        }
        if app.configured && !app.frame_pending && engine.due() {
            engine.render(&mut app, &qh)?;
        } else if app.configured
            && !app.frame_pending
            && engine.tick_mode
            && engine
                .planned
                .is_some_and(|p| p - mono_ms() > 1000.0 / Engine::refresh_hz(&app) * 1.5)
        {
            // Follow the compositor's refresh between frames, so the next
            // frame starts right after the vblank before its target.
            if let Some(layer) = &app.layer {
                layer.wl_surface().frame(&qh, layer.wl_surface().clone());
                layer.wl_surface().commit();
                app.frame_pending = true;
                app.tick_pending = true;
            }
        }
        if app.presented && !announced {
            announced = true;
            if options.ready {
                use std::io::Write;
                let mut out = std::io::stdout();
                let _ = out.write_all(b"ready\n");
                let _ = out.flush();
            }
        }
        connection.flush().map_err(|e| format!("Wayland: {e}"))?;

        let guard = queue.prepare_read();
        let wayland_fd = guard.as_ref().map_or(-1, |g| g.connection_fd().as_raw_fd());
        let mut fds = vec![libc::pollfd {
            fd: wayland_fd,
            events: libc::POLLIN,
            revents: 0,
        }];
        server.poll_fds(&mut fds);
        if let Some(h) = &hyprland {
            fds.push(libc::pollfd {
                fd: h.fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
        let timeout = if guard.is_none() {
            Some(Duration::ZERO)
        } else if app.tick_pending {
            // A compositor that does not repaint for an empty commit must not
            // stall the scene.
            engine
                .planned
                .map(|p| Duration::from_secs_f64(((p - mono_ms()) / 1000.0).max(0.0)))
        } else if app.frame_pending || !app.configured {
            None
        } else if engine.dirty {
            Some(Duration::ZERO)
        } else {
            engine
                .next_at
                .map(|at| Duration::from_secs_f64(((at - mono_ms()) / 1000.0).max(0.0)))
        };
        poll(&mut fds, server.timeout(timeout));
        if let Some(guard) = guard
            && fds[0].revents & libc::POLLIN != 0
        {
            let _ = guard.read();
        }
        for (client, command) in server.receive() {
            if matches!(command, Ok(Command::Load(_) | Command::Fps(_))) {
                app.intervals.clear();
                app.last_present = None;
            }
            let reply = match command {
                Ok(command) => engine.command(&app, command),
                Err(error) => serde_json::json!({"ok": false, "error": error}),
            };
            server.reply(client, &reply);
        }
        if let Some(h) = &mut hyprland {
            for event in h.receive() {
                match event {
                    HyprEvent::Workspace(index) => engine.workspace(index),
                    HyprEvent::Coverage => {
                        if let Some(covered) = h.covered() {
                            engine.set_covered(covered);
                        }
                    }
                }
            }
        }
    }
    drop(idle_notification);
    server.remove();
    if let (Some(gpu), Some(stage)) = (&engine.gpu, engine.stage.take()) {
        stage.release(&gpu.renderer);
    }
    if let Some(gpu) = engine.gpu.take() {
        let _ = gpu
            .context
            .egl
            .destroy_surface(gpu.context.display, gpu.surface);
        // SAFETY: the GPU surface has been destroyed and this is the sole release of its owned EGL window.
        unsafe {
            wayland_sys::ffi_dispatch!(wayland_egl_handle, wl_egl_window_destroy, gpu.window);
        };
    }
    Ok(())
}
