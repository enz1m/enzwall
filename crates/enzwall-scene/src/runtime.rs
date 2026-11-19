use crate::{Motion, Property, Props, Scene, Shape, Source, Trigger, Wave};

const TRIGGERS: usize = 6;

fn slot(trigger: Trigger) -> usize {
    match trigger {
        Trigger::Loop => 0,
        Trigger::Start => 1,
        Trigger::Unlock => 2,
        Trigger::Wake => 3,
        Trigger::Sleep => 4,
        Trigger::Workspace => 5,
        Trigger::Reading(_) => 0,
    }
}

const READING_EASE: f64 = 1.2;

#[derive(Clone, Copy, Debug)]
struct Reading {
    from: f32,
    to: f32,
    started: f64,
}

impl Reading {
    fn at(self, now: f64) -> (f32, bool) {
        let t = ((now - self.started) / READING_EASE).clamp(0.0, 1.0) as f32;
        (
            self.from + (self.to - self.from) * crate::Easing::InOutSine.apply(t),
            t < 1.0 && self.from != self.to,
        )
    }
}

/// Everything that changes while a scene runs. Times are seconds on any
/// monotonic clock chosen by the caller.
#[derive(Clone, Debug)]
pub struct Runtime {
    fired: [Option<f64>; TRIGGERS],
    paused_total: f64,
    paused_at: Option<f64>,
    camera_from: f32,
    camera_to: f32,
    camera_start: f64,
    started: f64,
    readings: [Option<Reading>; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerState {
    pub props: Props,
    /// Parallax offset as a fraction of the output width.
    pub shift: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Evaluated {
    pub layers: Vec<LayerState>,
    /// An event transition or the parallax camera is still moving.
    pub transitioning: bool,
    /// Looping motion is running and needs frames at the scene's fps.
    pub looping: bool,
    /// Opacity of the sleep veil drawn over every layer.
    pub dim: f32,
    /// Looping time for layers that animate themselves (particles, graphs).
    pub loop_time: f64,
}

const SLEEP_FADE: f32 = 1.5;
const WAKE_FADE: f32 = 0.8;

impl Runtime {
    pub fn new(now: f64) -> Self {
        let mut runtime = Self {
            fired: [None; TRIGGERS],
            paused_total: 0.0,
            paused_at: None,
            camera_from: 0.0,
            camera_to: 0.0,
            camera_start: now,
            started: now,
            readings: [None; 4],
        };
        runtime.fire(Trigger::Start, now);
        runtime
    }

    pub fn fire(&mut self, trigger: Trigger, now: f64) {
        if !matches!(trigger, Trigger::Loop | Trigger::Reading(_)) {
            self.fired[slot(trigger)] = Some(now);
        }
    }

    pub fn fired(&self, trigger: Trigger) -> Option<f64> {
        match trigger {
            Trigger::Loop | Trigger::Reading(_) => None,
            other => self.fired[slot(other)],
        }
    }

    /// Records a live reading (0–1). The first sample applies at once; later
    /// ones ease in so bound layers glide rather than jump.
    pub fn set_reading(&mut self, source: Source, value: f32, now: f64) {
        let value = value.clamp(0.0, 1.0);
        let slot = &mut self.readings[source.index()];
        *slot = Some(match *slot {
            None => Reading {
                from: value,
                to: value,
                started: now,
            },
            Some(r) if (r.to - value).abs() < 0.004 => r,
            Some(r) => Reading {
                from: r.at(now).0,
                to: value,
                started: now,
            },
        });
    }

    pub fn reading(&self, source: Source, now: f64) -> Option<(f32, bool)> {
        self.readings[source.index()].map(|r| r.at(now))
    }

    /// Seconds of looping time, which stops while loops are paused.
    pub fn loop_seconds(&self, now: f64) -> f64 {
        self.loop_time(now)
    }

    /// Moves the parallax camera to a desktop (0-based) and fires `workspace`.
    pub fn workspace(&mut self, scene: &Scene, index: u32, now: f64) {
        let current = self.camera(scene, now).0;
        self.camera_from = current;
        self.camera_to = index as f32;
        self.camera_start = now;
        self.fire(Trigger::Workspace, now);
    }

    /// Places the camera on a desktop without animating, e.g. at startup.
    pub fn jump(&mut self, index: u32, now: f64) {
        self.camera_from = index as f32;
        self.camera_to = index as f32;
        self.camera_start = now;
    }

    pub fn desktop(&self) -> u32 {
        self.camera_to.max(0.0) as u32
    }

    fn camera(&self, scene: &Scene, now: f64) -> (f32, bool) {
        let last = (scene.parallax.range - 1) as f32;
        let (from, to) = (
            self.camera_from.clamp(0.0, last),
            self.camera_to.clamp(0.0, last),
        );
        let t = ((now - self.camera_start) as f32 / scene.parallax.duration).clamp(0.0, 1.0);
        let e = scene.parallax.easing.apply(t);
        (from + (to - from) * e, t < 1.0 && from != to)
    }

    pub fn pause_loops(&mut self, now: f64) {
        self.paused_at.get_or_insert(now);
    }

    pub fn resume_loops(&mut self, now: f64) {
        if let Some(at) = self.paused_at.take() {
            self.paused_total += (now - at).max(0.0);
        }
    }

    pub fn loops_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    fn loop_time(&self, now: f64) -> f64 {
        let end = self.paused_at.unwrap_or(now);
        (end - self.started - self.paused_total).max(0.0)
    }
}

fn wave(shape: Wave, u: f64) -> f32 {
    let f = u - u.floor();
    (match shape {
        Wave::Sine => (u * std::f64::consts::TAU).sin(),
        Wave::Triangle => 4.0 * ((f - 0.25).rem_euclid(1.0) - 0.5).abs() - 1.0,
        Wave::Saw => 2.0 * f - 1.0,
        Wave::Scroll => 0.0,
    }) as f32
}

fn transition(
    runtime: &Runtime,
    motions: &[Motion],
    property: Property,
    now: f64,
) -> Option<(f32, bool)> {
    let (motion, fired) = motions
        .iter()
        .filter(|m| {
            m.property == property && !matches!(m.trigger, Trigger::Loop | Trigger::Reading(_))
        })
        .filter_map(|m| runtime.fired(m.trigger).map(|at| (m, at)))
        .fold(None, |best: Option<(&Motion, f64)>, (m, at)| match best {
            Some((_, best_at)) if best_at > at => best,
            _ => Some((m, at)),
        })?;
    let Shape::Transition {
        from,
        to,
        duration,
        delay,
        easing,
        hold,
    } = motion.shape
    else {
        return None;
    };
    let local = (now - fired) as f32 - delay;
    let neutral = if property.multiplicative() { 1.0 } else { 0.0 };
    Some(if local < 0.0 {
        (from, true)
    } else if duration <= 0.0 || local >= duration {
        (if hold { to } else { neutral }, false)
    } else {
        (from + (to - from) * easing.apply(local / duration), true)
    })
}

/// `aspect` is the output's width divided by its height.
pub fn evaluate(scene: &Scene, runtime: &Runtime, now: f64, aspect: f32) -> Evaluated {
    let (camera, mut transitioning) = runtime.camera(scene, now);
    let loop_time = runtime.loop_time(now);
    let looping = !runtime.loops_paused() && scene.has_loops();
    let layers = scene
        .layers
        .iter()
        .map(|layer| {
            let mut props = layer.base;
            for property in [
                Property::X,
                Property::Y,
                Property::Scale,
                Property::Rotation,
                Property::Opacity,
                Property::Phase,
            ] {
                let mut value = match property {
                    Property::X => props.x,
                    Property::Y => props.y,
                    Property::Scale => props.scale,
                    Property::Rotation => props.rotation,
                    Property::Opacity => props.opacity,
                    Property::Phase => props.phase,
                };
                for motion in layer.motions.iter().filter(|m| m.property == property) {
                    if let Shape::Loop {
                        wave: Wave::Scroll,
                        period,
                        amplitude,
                        phase,
                        ..
                    } = motion.shape
                    {
                        let cycles = f64::from(amplitude)
                            * (loop_time / f64::from(period) + f64::from(phase));
                        value += cycles.rem_euclid(1.0) as f32;
                        continue;
                    }
                    if let Shape::Loop {
                        wave: shape,
                        period,
                        amplitude,
                        phase,
                        height_units,
                    } = motion.shape
                    {
                        let w = wave(shape, loop_time / f64::from(period) + f64::from(phase));
                        let amplitude = if height_units {
                            amplitude / aspect.max(0.01)
                        } else {
                            amplitude
                        };
                        if property.multiplicative() {
                            value *= 1.0 + amplitude * w;
                        } else {
                            value += amplitude * w;
                        }
                    }
                }
                for motion in layer.motions.iter().filter(|m| m.property == property) {
                    let (Trigger::Reading(source), Shape::Transition { from, to, .. }) =
                        (motion.trigger, motion.shape)
                    else {
                        continue;
                    };
                    // Easing between readings is paced by the renderer's gauge
                    // rate, not the transition rate.
                    let Some((reading, _)) = runtime.reading(source, now) else {
                        continue;
                    };
                    let mapped = from + (to - from) * reading;
                    if property.multiplicative() {
                        value *= mapped;
                    } else {
                        value += mapped;
                    }
                }
                if let Some((delta, moving)) = transition(runtime, &layer.motions, property, now) {
                    transitioning |= moving;
                    if property.multiplicative() {
                        value *= delta;
                    } else {
                        value += delta;
                    }
                }
                match property {
                    Property::X => props.x = value,
                    Property::Y => props.y = value,
                    Property::Scale => props.scale = value.max(0.0),
                    Property::Rotation => props.rotation = value,
                    Property::Opacity => props.opacity = value.clamp(0.0, 1.0),
                    Property::Phase => props.phase = value,
                }
            }
            LayerState {
                props,
                shift: -camera * layer.depth * scene.parallax.shift,
            }
        })
        .collect();
    let awake_at = [Trigger::Wake, Trigger::Unlock]
        .iter()
        .filter_map(|t| runtime.fired(*t))
        .fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b))));
    let dim = match (runtime.fired(Trigger::Sleep), awake_at) {
        (Some(slept), awake) if awake.is_none_or(|w| w < slept) => {
            let t = ((now - slept) as f32 / SLEEP_FADE).clamp(0.0, 1.0);
            transitioning |= t < 1.0;
            scene.sleep_dim * crate::Easing::InOutSine.apply(t)
        }
        (Some(slept), Some(woke)) => {
            let t = ((now - woke) as f32 / WAKE_FADE).clamp(0.0, 1.0);
            let from = scene.sleep_dim
                * crate::Easing::InOutSine
                    .apply(((woke - slept) as f32 / SLEEP_FADE).clamp(0.0, 1.0));
            transitioning |= t < 1.0;
            from * (1.0 - crate::Easing::OutCubic.apply(t))
        }
        _ => 0.0,
    };
    Evaluated {
        layers,
        transitioning,
        looping,
        dim,
        loop_time,
    }
}
