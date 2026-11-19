use crate::file::LayerFile;
use crate::{Layer, MAX_MOTIONS, Motion, Props, Rgba, Template, range};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Add,
    /// Darkens like overprinted ink or a paper texture.
    Multiply,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// `corner` rounds the rectangle, relative to the output height.
    Solid {
        color: Rgba,
        corner: f32,
    },
    Gradient {
        stops: Vec<Rgba>,
        angle: f32,
        radial: bool,
    },
    /// `stretch` widens the circle into an ellipse (width ÷ height).
    Orb {
        color: Rgba,
        softness: f32,
        stretch: f32,
    },
    /// Arc angles are turns; `bind` drives the sweep from a live reading and `flat` squares the
    /// ends.
    Ring {
        color: Rgba,
        thickness: f32,
        softness: f32,
        sweep: f32,
        start: f32,
        bind: Option<Source>,
        track: Option<Rgba>,
        flat: bool,
    },
    /// `shade` fades the filled side from `color` at the curve to `shade`.
    Wave {
        color: Rgba,
        shade: Option<Rgba>,
        amplitude: f32,
        frequency: f32,
        detail: f32,
        softness: f32,
        fill: WaveFill,
    },
    /// `repeat` tiles the picture across the layer in both directions.
    Image {
        source: String,
        fit: Fit,
        tile: bool,
        repeat: (f32, f32),
    },
    Text(Text),
    Particles(Particles),
    /// A perspective floor of lines from the layer's top edge (the horizon)
    /// to its bottom; `phase` moves it toward the viewer.
    Grid {
        color: Rgba,
        density: f32,
        columns: f32,
        thickness: f32,
        fade: f32,
    },
    /// Recent history of a reading as a line (and optional fill).
    Graph {
        source: Source,
        color: Rgba,
        fill: Option<Rgba>,
        thickness: f32,
        glow: f32,
        samples: u32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub template: Template,
    pub font: Option<String>,
    pub size: f32,
    pub color: Rgba,
    pub align: Align,
    /// Extra space between letters, in em.
    pub tracking: f32,
    pub case: Case,
    /// Outline colour and width in em.
    pub outline: Option<(Rgba, f32)>,
    pub shadow: Option<Shadow>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub color: Rgba,
    /// Offset and blur radius, in em.
    pub x: f32,
    pub y: f32,
    pub blur: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    AsIs,
    Upper,
    Lower,
}

/// Many small glows (or one sprite image repeated) drifting through `area`.
/// Positions are derived from `seed`, so a scene looks the same every run.
#[derive(Clone, Debug, PartialEq)]
pub struct Particles {
    pub count: u32,
    /// Left, top, right and bottom, in output fractions.
    pub area: [f32; 4],
    /// Smallest and largest radius, relative to the output height.
    pub size: (f32, f32),
    /// Drift in output fractions per second.
    pub velocity: (f32, f32),
    /// 0 keeps every particle at `velocity`; 1 varies speed widely.
    pub jitter: f32,
    pub wobble: f32,
    pub wobble_period: f32,
    /// How much each particle's brightness pulses (0–1).
    pub twinkle: f32,
    pub colors: Vec<Rgba>,
    pub softness: f32,
    pub stretch: f32,
    pub seed: u32,
    pub sprite: Option<String>,
    /// Degrees per second for sprites.
    pub spin: f32,
}

/// Live readings a layer can follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Cpu,
    Mem,
    Battery,
    /// Local time of day, 0 at midnight to 1.
    Day,
}

impl Source {
    pub const ALL: [Self; 4] = [Self::Cpu, Self::Mem, Self::Battery, Self::Day];

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "cpu" => Self::Cpu,
            "mem" => Self::Mem,
            "battery" => Self::Battery,
            "day" => Self::Day,
            _ => return None,
        })
    }

    pub fn index(self) -> usize {
        match self {
            Self::Cpu => 0,
            Self::Mem => 1,
            Self::Battery => 2,
            Self::Day => 3,
        }
    }
}

/// Which side of a wave's curve is painted. `Band` glows on both sides;
/// `Rise` fades upward over `softness`, like a light curtain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaveFill {
    Down,
    Up,
    Band,
    Rise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Cover,
    Contain,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

fn asset(source: &str) -> Result<String, String> {
    let path = std::path::Path::new(source);
    if source.len() > 128
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        || path.extension().and_then(|e| e.to_str()) != Some("png")
    {
        return Err("image sources must be relative .png files inside the scene".into());
    }
    Ok(source.into())
}

fn pair(
    name: &str,
    value: Option<[f32; 2]>,
    default: (f32, f32),
    low: f32,
    high: f32,
) -> Result<(f32, f32), String> {
    let [a, b] = value.unwrap_or([default.0, default.1]);
    Ok((range(name, a, low, high)?, range(name, b, low, high)?))
}

impl LayerFile {
    fn color(&self) -> Result<Rgba, String> {
        Rgba::parse(self.color.as_deref().unwrap_or("#ffffff"))
    }

    fn colors(&self) -> Result<Vec<Rgba>, String> {
        self.colors
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|c| Rgba::parse(c))
            .collect()
    }

    fn source(&self) -> Result<Option<Source>, String> {
        self.bind
            .as_deref()
            .map(|name| {
                Source::parse(name)
                    .ok_or_else(|| format!("unknown reading {name:?} (cpu, mem, battery, day)"))
            })
            .transpose()
    }

    pub(crate) fn validate(self) -> Result<Layer, String> {
        let inferred = match (
            self.text.as_deref().is_some_and(|s| s.contains("{time")),
            self.text.as_deref().is_some_and(|s| s.contains("{date")),
        ) {
            (true, true) => "clock_date",
            (true, false) => "clock",
            (false, true) => "date",
            _ => "decoration",
        };
        let role = match self.role.as_deref().unwrap_or(inferred) {
            "clock" => crate::LayerRole::Clock,
            "date" => crate::LayerRole::Date,
            "clock_date" => crate::LayerRole::ClockDate,
            "decoration" => crate::LayerRole::Decoration,
            _ => return Err("role must be clock, date, clock_date or decoration".into()),
        };
        if self.font.as_ref().is_some_and(|f| {
            f.len() > 128 || f.contains('/') || f.contains('\\') || f.chars().any(char::is_control)
        }) {
            return Err("font must be a family or file stem, not a path".into());
        }

        let kind = match self.kind.as_str() {
            "solid" => Kind::Solid {
                color: self.color()?,
                corner: range("corner", self.corner.unwrap_or(0.0), 0.0, 1.0)?,
            },
            "gradient" => {
                let stops = self.colors()?;
                if !(2..=4).contains(&stops.len()) {
                    return Err("gradient needs 2–4 colors".into());
                }
                Kind::Gradient {
                    stops,
                    angle: range("angle", self.angle.unwrap_or(90.0), -360.0, 360.0)?,
                    radial: self.radial.unwrap_or(false),
                }
            }
            "orb" => Kind::Orb {
                color: self.color()?,
                softness: range("softness", self.softness.unwrap_or(1.0), 0.0, 1.0)?,
                stretch: range("stretch", self.stretch.unwrap_or(1.0), 0.05, 40.0)?,
            },
            "ring" => Kind::Ring {
                color: self.color()?,
                thickness: range("thickness", self.thickness.unwrap_or(0.1), 0.001, 1.0)?,
                softness: range("softness", self.softness.unwrap_or(0.3), 0.0, 1.0)?,
                sweep: range("sweep", self.sweep.unwrap_or(1.0), 0.0, 1.0)?,
                start: range("start", self.start.unwrap_or(0.0), -1.0, 1.0)?,
                bind: match self.source()? {
                    Some(Source::Day) => return Err("rings bind cpu, mem or battery".into()),
                    other => other,
                },
                track: self.track.as_deref().map(Rgba::parse).transpose()?,
                flat: match self.caps.as_deref().unwrap_or("round") {
                    "round" => false,
                    "flat" => true,
                    _ => return Err("caps must be round or flat".into()),
                },
            },
            "wave" => Kind::Wave {
                color: self.color()?,
                shade: self.shade.as_deref().map(Rgba::parse).transpose()?,
                amplitude: range("amplitude", self.amplitude.unwrap_or(0.05), 0.0, 1.0)?,
                frequency: range("frequency", self.frequency.unwrap_or(1.0), 0.0, 32.0)?,
                detail: range("detail", self.detail.unwrap_or(0.3), 0.0, 1.0)?,
                softness: range("softness", self.softness.unwrap_or(0.004), 0.0005, 0.5)?,
                fill: match self.fill.as_deref().unwrap_or("down") {
                    "down" => WaveFill::Down,
                    "up" => WaveFill::Up,
                    "band" => WaveFill::Band,
                    "rise" => WaveFill::Rise,
                    _ => return Err("fill must be down, up, band or rise".into()),
                },
            },
            "image" => Kind::Image {
                source: asset(self.source.as_deref().ok_or("image needs source")?)?,
                fit: match self.fit.as_deref().unwrap_or("cover") {
                    "cover" => Fit::Cover,
                    "contain" => Fit::Contain,
                    "stretch" => Fit::Stretch,
                    _ => return Err("fit must be cover, contain or stretch".into()),
                },
                tile: self.tile.unwrap_or(false),
                repeat: pair("repeat", self.repeat, (1.0, 1.0), 0.01, 512.0)?,
            },
            "text" => Kind::Text(Text {
                template: Template::parse(self.text.as_deref().ok_or("text needs text")?)?,
                font: self.font.clone(),
                size: range("size", self.size.unwrap_or(0.05), 0.005, 0.6)?,
                color: self.color()?,
                align: match self.align.as_deref().unwrap_or("center") {
                    "left" => Align::Left,
                    "center" => Align::Center,
                    "right" => Align::Right,
                    _ => return Err("align must be left, center or right".into()),
                },
                tracking: range("tracking", self.tracking.unwrap_or(0.0), -0.2, 2.0)?,
                case: match self.case.as_deref().unwrap_or("as-is") {
                    "as-is" => Case::AsIs,
                    "upper" => Case::Upper,
                    "lower" => Case::Lower,
                    _ => return Err("case must be as-is, upper or lower".into()),
                },
                outline: match &self.outline {
                    Some(color) => Some((
                        Rgba::parse(color)?,
                        range(
                            "outline_width",
                            self.outline_width.unwrap_or(0.06),
                            0.005,
                            0.5,
                        )?,
                    )),
                    None => None,
                },
                shadow: match &self.shadow {
                    Some(color) => {
                        let (x, y) =
                            pair("shadow_offset", self.shadow_offset, (0.0, 0.04), -1.0, 1.0)?;
                        Some(Shadow {
                            color: Rgba::parse(color)?,
                            x,
                            y,
                            blur: range("shadow_blur", self.shadow_blur.unwrap_or(0.06), 0.0, 1.0)?,
                        })
                    }
                    None => None,
                },
            }),
            "particles" => {
                let mut colors = self.colors()?;
                if colors.is_empty() {
                    colors.push(self.color()?);
                }
                if colors.len() > 6 {
                    return Err("particles take at most 6 colors".into());
                }
                let area = self.area.unwrap_or([0.0, 0.0, 1.0, 1.0]);
                for v in area {
                    range("area", v, -4.0, 8.0)?;
                }
                if area[2] <= area[0] || area[3] <= area[1] {
                    return Err(
                        "area is [left, top, right, bottom] with right > left and bottom > top"
                            .into(),
                    );
                }
                Kind::Particles(Particles {
                    count: self.count.unwrap_or(40).clamp(1, 160),
                    area,
                    size: pair("size", self.sizes, (0.002, 0.006), 0.0002, 0.5)?,
                    velocity: pair("velocity", self.velocity, (0.0, 0.0), -2.0, 2.0)?,
                    jitter: range("jitter", self.jitter.unwrap_or(0.5), 0.0, 1.0)?,
                    wobble: range("wobble", self.wobble.unwrap_or(0.0), 0.0, 0.5)?,
                    wobble_period: range(
                        "wobble_period",
                        self.wobble_period.unwrap_or(6.0),
                        0.5,
                        600.0,
                    )?,
                    twinkle: range("twinkle", self.twinkle.unwrap_or(0.0), 0.0, 1.0)?,
                    colors,
                    softness: range("softness", self.softness.unwrap_or(0.8), 0.0, 1.0)?,
                    stretch: range("stretch", self.stretch.unwrap_or(1.0), 0.05, 40.0)?,
                    seed: self.seed.unwrap_or(1),
                    sprite: self.source.as_deref().map(asset).transpose()?,
                    spin: range("spin", self.spin.unwrap_or(0.0), -720.0, 720.0)?,
                })
            }
            "grid" => Kind::Grid {
                color: self.color()?,
                density: range("density", self.density.unwrap_or(6.0), 0.5, 64.0)?,
                columns: range("columns", self.columns.unwrap_or(12.0), 1.0, 128.0)?,
                thickness: range("thickness", self.thickness.unwrap_or(1.5), 0.25, 12.0)?,
                fade: range("fade", self.fade.unwrap_or(0.35), 0.0, 1.0)?,
            },
            "graph" => Kind::Graph {
                source: match self.source()? {
                    Some(source @ (Source::Cpu | Source::Mem)) => source,
                    _ => return Err("graphs bind cpu or mem".into()),
                },
                color: self.color()?,
                fill: self.shade.as_deref().map(Rgba::parse).transpose()?,
                thickness: range("thickness", self.thickness.unwrap_or(2.0), 0.5, 16.0)?,
                glow: range("glow", self.glow.unwrap_or(0.0), 0.0, 1.0)?,
                samples: self.count.unwrap_or(60).clamp(8, 64),
            },
            other => return Err(format!("unknown layer kind {other:?}")),
        };
        let full = matches!(
            kind,
            Kind::Solid { .. }
                | Kind::Gradient { .. }
                | Kind::Image { .. }
                | Kind::Grid { .. }
                | Kind::Graph { .. }
        );
        let base = Props {
            x: range("x", self.x.unwrap_or(0.5), -2.0, 3.0)?,
            y: range("y", self.y.unwrap_or(0.5), -2.0, 3.0)?,
            w: range(
                "w",
                self.w.unwrap_or(if full { 1.0 } else { 0.4 }),
                0.0,
                8.0,
            )?,
            h: range(
                "h",
                self.h.unwrap_or(if full { 1.0 } else { 0.4 }),
                0.0,
                8.0,
            )?,
            radius: range("radius", self.radius.unwrap_or(0.25), 0.0, 4.0)?,
            rotation: range("rotation", self.rotation.unwrap_or(0.0), -3600.0, 3600.0)?,
            scale: range("scale", self.scale.unwrap_or(1.0), 0.0, 16.0)?,
            opacity: range("opacity", self.opacity.unwrap_or(1.0), 0.0, 1.0)?,
            phase: range("phase", self.phase.unwrap_or(0.0), -1000.0, 1000.0)?,
        };
        if self.motion.len() > MAX_MOTIONS {
            return Err(format!("at most {MAX_MOTIONS} motions per layer"));
        }
        let motions = self
            .motion
            .into_iter()
            .map(super::file::MotionFile::validate)
            .collect::<Result<Vec<Motion>, _>>()?;
        if let Some(id) = &self.id
            && (id.is_empty()
                || id.len() > 32
                || !id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        {
            return Err("id must be 1–32 of [A-Za-z0-9_-]".into());
        }
        Ok(Layer {
            role,
            id: self.id,
            kind,
            base,
            depth: range("depth", self.depth.unwrap_or(0.0), -4.0, 4.0)?,
            pivot: pair("pivot", self.pivot, (0.5, 0.5), -2.0, 3.0)?,
            blend: match self.blend.as_deref().unwrap_or("normal") {
                "normal" => Blend::Normal,
                "add" => Blend::Add,
                "multiply" => Blend::Multiply,
                _ => return Err("blend must be normal, add or multiply".into()),
            },
            motions,
        })
    }
}
