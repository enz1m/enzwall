//! Scene parsing, validation and animation evaluation without I/O or rendering.
#![allow(
    missing_docs,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "This application-internal scene model implements the documented wallpaper package format; fields do not form a supported external Rust API."
)]

mod color;
mod easing;
mod file;
mod layer;
mod runtime;
mod template;

pub use color::Rgba;
pub use easing::Easing;
pub use layer::{Align, Blend, Case, Fit, Kind, Particles, Shadow, Source, Text, WaveFill};
pub use runtime::{Evaluated, LayerState, Runtime, evaluate};
pub use template::{Cadence, Part, Template};

use file::{MotionFile, SceneFile};

pub const FORMAT: u32 = 1;
pub const MAX_LAYERS: usize = 32;
pub const MAX_MOTIONS: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub name: String,
    pub description: String,
    pub author: String,
    pub fps: u32,
    pub transition_fps: u32,
    pub background: Rgba,
    pub render_scale: f32,
    pub parallax: Parallax,
    pub idle_timeout: u32,
    /// Darkening applied over the whole scene while asleep (0 disables).
    pub sleep_dim: f32,
    pub layers: Vec<Layer>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parallax {
    /// Fraction of the output width a depth-1 layer moves per desktop.
    pub shift: f32,
    pub duration: f32,
    pub easing: Easing,
    /// Desktops the camera travels across; later desktops reuse the last view
    /// so finite layers never drift off screen.
    pub range: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayerRole {
    #[default]
    Decoration,
    Clock,
    Date,
    ClockDate,
}
impl LayerRole {
    pub fn hidden(self, clock: bool, date: bool) -> bool {
        match self {
            Self::Decoration => false,
            Self::Clock => clock,
            Self::Date => date,
            Self::ClockDate => clock || date,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub role: LayerRole,
    pub id: Option<String>,
    pub kind: Kind,
    pub base: Props,
    pub depth: f32,
    /// Rotation and scale origin, in fractions of the layer's own box.
    pub pivot: (f32, f32),
    pub blend: Blend,
    pub motions: Vec<Motion>,
}

/// Geometry is in output fractions: `x`/`y` are the layer centre, `w`/`h` its
/// extent and `radius` is relative to the output height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Props {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
    pub rotation: f32,
    pub scale: f32,
    pub opacity: f32,
    pub phase: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Property {
    X,
    Y,
    Scale,
    Rotation,
    Opacity,
    Phase,
}

impl Property {
    pub fn multiplicative(self) -> bool {
        matches!(self, Self::Scale | Self::Opacity)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trigger {
    Loop,
    Start,
    Unlock,
    Wake,
    Sleep,
    Workspace,
    /// Follows a live reading: `from` at 0 %, `to` at 100 %.
    Reading(Source),
}

impl Trigger {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "loop" => Self::Loop,
            "start" => Self::Start,
            "unlock" => Self::Unlock,
            "wake" => Self::Wake,
            "sleep" => Self::Sleep,
            "workspace" => Self::Workspace,
            other => Self::Reading(Source::parse(other)?),
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Loop => "loop",
            Self::Start => "start",
            Self::Unlock => "unlock",
            Self::Wake => "wake",
            Self::Sleep => "sleep",
            Self::Workspace => "workspace",
            Self::Reading(Source::Cpu) => "cpu",
            Self::Reading(Source::Mem) => "mem",
            Self::Reading(Source::Battery) => "battery",
            Self::Reading(Source::Day) => "day",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Triangle,
    Saw,
    /// Endless drift of `amplitude` per `period`; only for `phase`, which is
    /// cyclic, so it never visibly wraps.
    Scroll,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// `height_units` measures an `x` amplitude in output heights, so paired
    /// `x`/`y` loops trace circles on any aspect ratio.
    Loop {
        wave: Wave,
        period: f32,
        amplitude: f32,
        phase: f32,
        height_units: bool,
    },
    Transition {
        from: f32,
        to: f32,
        duration: f32,
        delay: f32,
        easing: Easing,
        hold: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub property: Property,
    pub trigger: Trigger,
    pub shape: Shape,
}

impl Scene {
    pub fn parse(source: &str) -> Result<Self, String> {
        if source.len() > 256 * 1024 {
            return Err("scene.toml exceeds 256 KiB".into());
        }
        let file: SceneFile = toml::from_str(source).map_err(|error| error.to_string())?;
        file.validate()
    }

    pub fn has_loops(&self) -> bool {
        self.has_ambient()
            || self
                .layers
                .iter()
                .any(|layer| layer.motions.iter().any(|m| m.trigger == Trigger::Loop))
    }

    /// Live readings the scene shows, so the renderer samples only those.
    pub fn readings(&self) -> Vec<Source> {
        let mut out = Vec::new();
        for layer in &self.layers {
            let mut add = |s: Source| {
                if !out.contains(&s) {
                    out.push(s);
                }
            };
            match &layer.kind {
                Kind::Ring { bind: Some(s), .. } | Kind::Graph { source: s, .. } => add(*s),
                _ => {}
            }
            for motion in &layer.motions {
                if let Trigger::Reading(s) = motion.trigger {
                    add(s);
                }
            }
        }
        out
    }

    pub fn has_gauges(&self) -> bool {
        !self.readings().is_empty()
    }

    /// Layers that move on their own every frame (particles, graphs).
    pub fn has_ambient(&self) -> bool {
        self.layers
            .iter()
            .any(|l| matches!(l.kind, Kind::Particles(_) | Kind::Graph { .. }))
    }

    pub fn images(&self) -> impl Iterator<Item = &str> {
        self.layers.iter().filter_map(|layer| match &layer.kind {
            Kind::Image { source, .. } => Some(source.as_str()),
            Kind::Particles(p) => p.sprite.as_deref(),
            _ => None,
        })
    }
}

pub(crate) fn range(name: &str, value: f32, low: f32, high: f32) -> Result<f32, String> {
    if value.is_finite() && (low..=high).contains(&value) {
        Ok(value)
    } else {
        Err(format!("{name} must be between {low} and {high}"))
    }
}

impl SceneFile {
    fn validate(self) -> Result<Scene, String> {
        if self.format != FORMAT {
            return Err(format!(
                "Unsupported scene format {} (expected {FORMAT})",
                self.format
            ));
        }
        if self.name.trim().is_empty() || self.name.len() > 64 {
            return Err("name must be 1–64 characters".into());
        }
        if self.layers.is_empty() || self.layers.len() > MAX_LAYERS {
            return Err(format!("A scene needs 1–{MAX_LAYERS} layers"));
        }
        let fps = self.fps.unwrap_or(30);
        let transition_fps = self.transition_fps.unwrap_or(60);
        if !(1..=60).contains(&fps) || !(1..=120).contains(&transition_fps) {
            return Err("fps must be 1–60 and transition_fps 1–120".into());
        }
        let parallax = self.parallax.unwrap_or_default();
        let parallax = Parallax {
            shift: range("parallax.shift", parallax.shift.unwrap_or(0.0), 0.0, 1.0)?,
            duration: range(
                "parallax.duration",
                parallax.duration.unwrap_or(0.7),
                0.05,
                5.0,
            )?,
            easing: Easing::parse(parallax.easing.as_deref().unwrap_or("in-out-cubic"))?,
            range: parallax.range.unwrap_or(5).clamp(1, 64),
        };
        let layers = self
            .layers
            .into_iter()
            .enumerate()
            .map(|(index, layer)| {
                layer
                    .validate()
                    .map_err(|e| format!("layer {}: {e}", index + 1))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Scene {
            name: self.name,
            description: self.description.unwrap_or_default(),
            author: self.author.unwrap_or_default(),
            fps,
            transition_fps,
            background: Rgba::parse(self.background.as_deref().unwrap_or("#000000"))?,
            render_scale: range("render_scale", self.render_scale.unwrap_or(1.0), 0.25, 1.0)?,
            parallax,
            idle_timeout: self.idle_timeout.unwrap_or(300).clamp(10, 86_400),
            sleep_dim: range("sleep_dim", self.sleep_dim.unwrap_or(0.6), 0.0, 1.0)?,
            layers,
        })
    }
}

impl MotionFile {
    fn validate(self) -> Result<Motion, String> {
        let property = match self.property.as_str() {
            "x" => Property::X,
            "y" => Property::Y,
            "scale" => Property::Scale,
            "rotation" => Property::Rotation,
            "opacity" => Property::Opacity,
            "phase" => Property::Phase,
            other => return Err(format!("unknown motion property {other:?}")),
        };
        let trigger = Trigger::parse(self.on.as_deref().unwrap_or("loop")).ok_or(
            "on must be loop, start, unlock, wake, sleep, workspace, cpu, mem, battery or day",
        )?;
        let shape = if trigger == Trigger::Loop {
            Shape::Loop {
                wave: match self.wave.as_deref().unwrap_or("sine") {
                    "sine" => Wave::Sine,
                    "triangle" => Wave::Triangle,
                    "saw" => Wave::Saw,
                    "scroll" if property == Property::Phase => Wave::Scroll,
                    "scroll" => return Err("wave = \"scroll\" only animates phase".into()),
                    _ => return Err("wave must be sine, triangle, saw or scroll".into()),
                },
                period: range("period", self.period.unwrap_or(20.0), 0.1, 3600.0)?,
                amplitude: range("amplitude", self.amplitude.unwrap_or(0.02), -1000.0, 1000.0)?,
                phase: range("phase", self.phase.unwrap_or(0.0), -1.0, 1.0)?,
                height_units: match self.unit.as_deref() {
                    None | Some("natural") => false,
                    Some("height") if property == Property::X => true,
                    _ => return Err("unit = \"height\" is only valid for x".into()),
                },
            }
        } else {
            let neutral = if property.multiplicative() { 1.0 } else { 0.0 };
            Shape::Transition {
                from: range("from", self.from.unwrap_or(neutral), -1000.0, 1000.0)?,
                to: range("to", self.to.unwrap_or(neutral), -1000.0, 1000.0)?,
                duration: range("duration", self.duration.unwrap_or(1.0), 0.0, 30.0)?,
                delay: range("delay", self.delay.unwrap_or(0.0), 0.0, 30.0)?,
                easing: Easing::parse(self.easing.as_deref().unwrap_or("out-cubic"))?,
                hold: self.hold.unwrap_or(false),
            }
        };
        Ok(Motion {
            property,
            trigger,
            shape,
        })
    }
}

#[cfg(test)]
mod tests;
