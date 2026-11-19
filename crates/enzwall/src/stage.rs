use crate::gl::{Area, GraphData, Renderer, Resources, TextTextures, Texture};
use crate::text::{Coverage, Fonts, blur, dilate, rasterize};
use crate::vars::Vars;
use enzwall_scene::{Cadence, Case, Evaluated, Kind, Runtime, Scene, Source, evaluate};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_IMAGE: u32 = 4096;
const GRAPH_STEP: f64 = 2.0;

pub(crate) fn read_scene(dir: &Path) -> Result<Scene, String> {
    let path = dir.join("scene.toml");
    let source = read_asset(&path, 256 * 1024, "Scene exceeds 256 KiB")?;
    let source = std::str::from_utf8(&source).map_err(|e| format!("{}: {e}", path.display()))?;
    Scene::parse(source).map_err(|e| format!("{}: {e}", path.display()))
}

fn open_asset(path: &Path) -> Result<File, String> {
    // A named pipe must fail validation without stalling the rendering loop.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err(format!("{} must be a regular file", path.display()));
    }
    Ok(file)
}

pub(crate) fn read_asset(path: &Path, maximum: usize, limit: &str) -> Result<Vec<u8>, String> {
    let file = open_asset(path)?;
    if file.metadata().map_err(|e| e.to_string())?.len() > maximum as u64 {
        return Err(limit.into());
    }
    let mut data = Vec::new();
    file.take(maximum as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if data.len() > maximum {
        return Err(limit.into());
    }
    Ok(data)
}

/// Decodes a PNG into premultiplied RGBA8.
pub(crate) fn decode_png(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(BufReader::new(open_asset(path)?));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let info = reader.info();
    if info.width == 0 || info.height == 0 || info.width > MAX_IMAGE || info.height > MAX_IMAGE {
        return Err(format!(
            "{}: images must be at most {MAX_IMAGE}×{MAX_IMAGE}",
            path.display()
        ));
    }
    let mut buffer = vec![0; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let pixels = &buffer[..frame.buffer_size()];
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(format!("{}: unexpanded palette", path.display())),
    };
    let premultiplied = rgba
        .chunks_exact(4)
        .flat_map(|p| {
            let a = u32::from(p[3]);
            [
                (u32::from(p[0]) * a / 255) as u8,
                (u32::from(p[1]) * a / 255) as u8,
                (u32::from(p[2]) * a / 255) as u8,
                p[3],
            ]
        })
        .collect();
    Ok((frame.width, frame.height, premultiplied))
}

struct History {
    samples: VecDeque<f32>,
    capacity: usize,
    pushed: f64,
}

pub(crate) struct Stage {
    pub scene: Scene,
    pub dir: PathBuf,
    pub runtime: Runtime,
    images: Vec<Option<Texture>>,
    texts: Vec<Option<TextTextures>>,
    strings: Vec<Option<(String, u32)>>,
    text_bytes: Vec<usize>,
    histories: Vec<Option<History>>,
    fonts: Fonts,
    readings: Vec<Source>,
    now: f64,
    pub last: Option<Evaluated>,
}

fn reading_name(source: Source) -> &'static str {
    match source {
        Source::Cpu => "cpu",
        Source::Mem => "mem",
        Source::Battery => "battery",
        Source::Day => "day",
    }
}

struct PendingImages<'a> {
    renderer: &'a Renderer,
    textures: Vec<Option<Texture>>,
}
impl Drop for PendingImages<'_> {
    fn drop(&mut self) {
        for texture in self.textures.drain(..).flatten() {
            self.renderer.delete(texture);
        }
    }
}

impl Stage {
    pub(crate) fn load(renderer: &Renderer, dir: &Path, now: f64) -> Result<Self, String> {
        Self::load_filtered(renderer, dir, now, false, false)
    }
    pub(crate) fn load_filtered(
        renderer: &Renderer,
        dir: &Path,
        now: f64,
        hide_clock: bool,
        hide_date: bool,
    ) -> Result<Self, String> {
        let mut scene = read_scene(dir)?;
        scene
            .layers
            .retain(|layer| !layer.role.hidden(hide_clock, hide_date));
        let mut images = PendingImages {
            renderer,
            textures: Vec::with_capacity(scene.layers.len()),
        };
        let mut asset_bytes = 0_usize;
        for layer in &scene.layers {
            let (source, repeat) = match &layer.kind {
                Kind::Image {
                    source,
                    tile,
                    repeat,
                    ..
                } => (Some(source), (*tile || repeat.0 != 1.0, repeat.1 != 1.0)),
                Kind::Particles(p) => (p.sprite.as_ref(), (false, false)),
                _ => (None, (false, false)),
            };
            images.textures.push(match source {
                Some(source) => {
                    let (w, h, pixels) = decode_png(&dir.join(source))?;
                    asset_bytes = asset_bytes
                        .checked_add(pixels.len())
                        .ok_or("Scene asset size overflow")?;
                    if asset_bytes > 64 * 1024 * 1024 {
                        return Err("Scene images exceed 64 MiB decoded".into());
                    }
                    Some(renderer.upload(w, h, &pixels, false, repeat)?)
                }
                None => None,
            });
        }
        let histories = scene
            .layers
            .iter()
            .map(|l| match l.kind {
                Kind::Graph { samples, .. } => Some(History {
                    samples: VecDeque::new(),
                    capacity: (samples as usize).min(63),
                    pushed: f64::NEG_INFINITY,
                }),
                _ => None,
            })
            .collect();
        let count = scene.layers.len();
        let readings = scene.readings();
        Ok(Self {
            scene,
            dir: dir.to_path_buf(),
            runtime: Runtime::new(now),
            images: std::mem::take(&mut images.textures),
            texts: (0..count).map(|_| None).collect(),
            strings: vec![None; count],
            text_bytes: vec![0; count],
            histories,
            fonts: Fonts::new(),
            readings,
            now,
            last: None,
        })
    }

    pub(crate) fn release(self, renderer: &Renderer) {
        for texture in self.images.into_iter().flatten() {
            renderer.delete(texture);
        }
        for text in self.texts.into_iter().flatten() {
            for texture in [Some(text.fill), text.outline, text.shadow]
                .into_iter()
                .flatten()
            {
                renderer.delete(texture);
            }
        }
    }

    /// Samples live readings, extends graph histories and re-rasterises text
    /// layers whose string or size changed; returns the changed text layers.
    pub(crate) fn refresh_texts(
        &mut self,
        renderer: &Renderer,
        vars: &mut Vars,
        height: u32,
        now: f64,
    ) -> Result<Vec<usize>, String> {
        self.now = now;
        for &source in &self.readings {
            let value = match source {
                Source::Day => Some(vars.day_fraction()),
                other => vars
                    .value(reading_name(other), None)
                    .parse::<f32>()
                    .ok()
                    .map(|v| v / 100.0),
            };
            if let Some(value) = value {
                self.runtime.set_reading(source, value, now);
            }
        }
        for (layer, history) in self.scene.layers.iter().zip(&mut self.histories) {
            let (Kind::Graph { source, .. }, Some(history)) = (&layer.kind, history) else {
                continue;
            };
            if now - history.pushed >= GRAPH_STEP
                && let Some((value, _)) = self.runtime.reading(*source, now)
            {
                if history.samples.is_empty() {
                    history
                        .samples
                        .extend(std::iter::repeat_n(value, history.capacity));
                }
                history.samples.push_back(value);
                while history.samples.len() > history.capacity + 1 {
                    history.samples.pop_front();
                }
                history.pushed =
                    if history.pushed.is_finite() && now - history.pushed < GRAPH_STEP * 2.0 {
                        history.pushed + GRAPH_STEP
                    } else {
                        now
                    };
            }
        }
        let mut changed = Vec::new();
        for (index, layer) in self.scene.layers.iter().enumerate() {
            let Kind::Text(text) = &layer.kind else {
                continue;
            };
            let mut string = text.template.render(|name, arg| vars.value(name, arg));
            match text.case {
                Case::Upper => string = string.to_uppercase(),
                Case::Lower => string = string.to_lowercase(),
                Case::AsIs => {}
            }
            let px = ((text.size * height as f32).round() as u32).max(4);
            if self.strings[index].as_ref() == Some(&(string.clone(), px)) {
                continue;
            }
            let em = px as f32;
            let outline_px = text.outline.map_or(0.0, |(_, width)| width * em);
            let shadow_reach = text.shadow.map_or(0.0, |s| s.blur * em * 2.0);
            let pad = outline_px + shadow_reach;
            let font = self.fonts.font(&self.dir, text.font.as_deref())?;
            let fill = rasterize(font, &string, em, text.tracking, pad)?;
            if vars.public
                && text.outline.is_some_and(|(_, width)| {
                    fill.pixels.len() as f64 * (2.0 * f64::from(width * em) + 1.0).powi(2)
                        > 128_000_000.0
                })
            {
                return Err("Public scene text outline exceeds its work budget".into());
            }
            let bytes = fill.pixels.len()
                * (1 + usize::from(text.outline.is_some()) + usize::from(text.shadow.is_some()));
            if self.text_bytes.iter().sum::<usize>() - self.text_bytes[index] + bytes
                > 32 * 1024 * 1024
            {
                return Err("Scene text exceeds 32 MiB".into());
            }
            self.text_bytes[index] = bytes;
            let outline = text.outline.map(|(_, width)| dilate(&fill, width * em));
            let shadow = text.shadow.map(|s| {
                let base = outline.as_ref().unwrap_or(&fill);
                if s.blur > 0.0 {
                    blur(base, s.blur * em)
                } else {
                    Coverage {
                        width: base.width,
                        height: base.height,
                        pixels: base.pixels.clone(),
                    }
                }
            });
            let upload =
                |c: &Coverage| renderer.upload(c.width, c.height, &c.pixels, true, (false, false));
            let textures = TextTextures {
                fill: upload(&fill)?,
                outline: outline.as_ref().map(upload).transpose()?,
                shadow: shadow.as_ref().map(upload).transpose()?,
                pad: (em * 0.1).ceil().max(2.0) + pad.ceil(),
            };
            if let Some(old) = self.texts[index].replace(textures) {
                for texture in [Some(old.fill), old.outline, old.shadow]
                    .into_iter()
                    .flatten()
                {
                    renderer.delete(texture);
                }
            }
            self.strings[index] = Some((string, px));
            changed.push(index);
        }
        Ok(changed)
    }

    /// A bound ring, graph or value-driven layer is still easing toward its
    /// latest reading.
    pub(crate) fn gauges_moving(&self) -> bool {
        self.readings.iter().any(|&s| {
            self.runtime
                .reading(s, self.now)
                .is_some_and(|(_, moving)| moving)
        })
    }

    pub(crate) fn evaluate(&mut self, now: f64, width: u32, height: u32) -> Evaluated {
        self.now = now;
        evaluate(
            &self.scene,
            &self.runtime,
            now,
            width as f32 / height.max(1) as f32,
        )
    }

    pub(crate) fn draw_state(
        &mut self,
        renderer: &Renderer,
        state: Evaluated,
        now: f64,
        width: u32,
        height: u32,
    ) -> Vec<Area> {
        let gauges: Vec<Option<f32>> = self
            .scene
            .layers
            .iter()
            .map(|l| match l.kind {
                Kind::Ring {
                    bind: Some(source), ..
                } => Some(self.runtime.reading(source, now).map_or(0.0, |(v, _)| v)),
                _ => None,
            })
            .collect();
        let graphs: Vec<Option<GraphData>> = self
            .histories
            .iter()
            .map(|h| {
                h.as_ref().map(|h| GraphData {
                    samples: h.samples.iter().copied().collect(),
                    scroll: ((now - h.pushed) / GRAPH_STEP).clamp(0.0, 1.0) as f32,
                })
            })
            .collect();
        let resources = Resources {
            images: &self.images,
            texts: &self.texts,
            gauges: &gauges,
            graphs: &graphs,
        };
        let areas = renderer.draw(&self.scene, &state, width, height, &resources);
        self.last = Some(state);
        areas
    }

    pub(crate) fn draw(
        &mut self,
        renderer: &Renderer,
        now: f64,
        width: u32,
        height: u32,
    ) -> Vec<Area> {
        let state = self.evaluate(now, width, height);
        self.draw_state(renderer, state, now, width, height)
    }

    /// Time until the next text layer or reading can change, from
    /// wall-clock milliseconds.
    pub(crate) fn next_text_change(&self, unix_ms: i64) -> Option<Duration> {
        let mut cadence = self
            .scene
            .layers
            .iter()
            .filter_map(|l| match &l.kind {
                Kind::Text(text) => Some(text.template.cadence()),
                _ => None,
            })
            .min();
        if !self.readings.is_empty() {
            let reading = if self.readings.iter().any(|s| *s != Source::Day) {
                Cadence::TwoSeconds
            } else {
                Cadence::Minute
            };
            cadence = Some(cadence.map_or(reading, |c| c.min(reading)));
        }
        let period: i64 = match cadence? {
            Cadence::Second => 1_000,
            Cadence::TwoSeconds => 2_000,
            Cadence::HalfMinute => 30_000,
            Cadence::Minute => 60_000,
            Cadence::Never => return None,
        };
        Some(Duration::from_millis(
            (period - unix_ms.rem_euclid(period) + 5) as u64,
        ))
    }
}
