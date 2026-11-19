//! Frame pacing: how much a scene visibly changed between two moments, and
//! which power profile decides the loop frame rate.

use enzwall_scene::{Evaluated, Kind, Scene};

/// Changes smaller than this (pixels, or 8-bit colour levels) are invisible,
/// so a frame showing only them is skipped.
pub(crate) const THRESHOLD: f32 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Smoothness {
    /// Loops at the scene's own frame cap.
    Balanced,
    /// Loops at the display's refresh rate.
    Smooth,
    /// Smooth on mains power, Balanced on battery.
    Auto,
}

impl Smoothness {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "balanced" => Self::Balanced,
            "smooth" => Self::Smooth,
            "auto" => Self::Auto,
            _ => return None,
        })
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Smooth => "smooth",
            Self::Auto => "auto",
        }
    }
}

pub(crate) fn on_mains() -> bool {
    let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") else {
        return true;
    };
    for entry in dir.flatten() {
        let path = entry.path();
        let read = |f: &str| {
            std::fs::read_to_string(path.join(f))
                .unwrap_or_default()
                .trim()
                .to_owned()
        };
        if read("type") == "Battery" && read("status") == "Discharging" {
            return false;
        }
    }
    true
}

/// Largest visible change of each layer between two evaluated states that
/// are `dt` seconds apart.
pub(crate) fn changes(
    scene: &Scene,
    a: &Evaluated,
    b: &Evaluated,
    dt: f32,
    w: f32,
    h: f32,
) -> Vec<f32> {
    scene
        .layers
        .iter()
        .zip(a.layers.iter().zip(&b.layers))
        .map(|(layer, (pa, pb))| {
            let (p, q) = (pa.props, pb.props);
            let size = match &layer.kind {
                Kind::Orb { stretch, .. } => 2.0 * q.radius * h * stretch.max(1.0),
                Kind::Ring { .. } => 2.0 * q.radius * h,
                Kind::Text(t) => t.size * h * 3.0,
                Kind::Wave { amplitude, .. } => w.max(amplitude * h),
                _ => (q.w * w).max(q.h * h),
            } * q.scale.max(p.scale);
            let travel = (((q.x + pb.shift) - (p.x + pa.shift)) * w)
                .abs()
                .max(((q.y - p.y) * h).abs());
            let zoom = (q.scale - p.scale).abs() * size;
            let turn = (q.rotation - p.rotation).abs().to_radians() * size * 0.5;
            let fade = (q.opacity - p.opacity).abs() * 255.0;
            let slide = match &layer.kind {
                Kind::Wave { frequency, .. } => {
                    // Phase wraps every whole cycle, so measure the short way round.
                    let d = (q.phase - p.phase).rem_euclid(1.0);
                    d.min(1.0 - d) * w / frequency.max(0.05)
                }
                Kind::Image { tile: true, .. } => (q.phase - p.phase).abs() * q.w * w,
                Kind::Grid { .. } => (q.phase - p.phase).abs() * h,
                _ => 0.0,
            };
            let ambient = match &layer.kind {
                Kind::Particles(pt) => {
                    let speed =
                        pt.velocity.0.abs().max(pt.velocity.1.abs()) * (1.0 + pt.jitter) * w;
                    let sway = pt.wobble * h * std::f32::consts::TAU / pt.wobble_period;
                    let flicker = pt.twinkle * 255.0 * std::f32::consts::TAU / 1.5 * q.opacity;
                    (speed + sway).max(flicker) * dt
                }
                Kind::Graph { samples, .. } => q.w * w / *samples as f32 * dt / 2.0,
                _ => 0.0,
            };
            travel.max(zoom).max(turn).max(fade).max(slide).max(ambient)
        })
        .collect()
}
