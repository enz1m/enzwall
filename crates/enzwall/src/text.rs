use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const FALLBACKS: &[&str] = &[
    "Inter-Regular",
    "Roboto-Regular",
    "NotoSans-Regular",
    "Cantarell-Regular",
    "DejaVuSans",
    "LiberationSans-Regular",
];

/// Loads each font once; `name` is a file stem such as `Roboto-Light` or a
/// `.ttf`/`.otf` inside the scene directory.
pub(crate) struct Fonts {
    index: Option<HashMap<String, PathBuf>>,
    loaded: HashMap<String, FontVec>,
}

fn scan(dir: &Path, depth: u32, out: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && depth < 4 {
            scan(&path, depth + 1, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("ttf" | "otf")
        ) && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            out.entry(stem.to_ascii_lowercase()).or_insert(path);
        }
    }
}

impl Fonts {
    pub(crate) fn new() -> Self {
        Self {
            index: None,
            loaded: HashMap::new(),
        }
    }

    fn resolve(&mut self, scene_dir: &Path, name: Option<&str>) -> Option<PathBuf> {
        if let Some(name) = name.filter(|n| !n.contains('/') && !n.starts_with('.')) {
            let shared = scene_dir.parent().map(|d| d.join("_shared/fonts"));
            for dir in std::iter::once(scene_dir.to_path_buf()).chain(shared) {
                for candidate in [
                    name.to_owned(),
                    format!("{name}.ttf"),
                    format!("{name}.otf"),
                ] {
                    let path = dir.join(&candidate);
                    if path.is_file() {
                        return Some(path);
                    }
                }
            }
        }
        let index = self.index.get_or_insert_with(|| {
            let mut out = HashMap::new();
            if let Some(home) = std::env::var_os("HOME") {
                scan(&Path::new(&home).join(".local/share/fonts"), 0, &mut out);
            }
            scan(Path::new("/usr/share/fonts"), 0, &mut out);
            out
        });
        name.into_iter()
            .chain(FALLBACKS.iter().copied())
            .find_map(|n| index.get(&n.to_ascii_lowercase()).cloned())
    }

    pub(crate) fn exists(&mut self, scene_dir: &Path, name: &str) -> bool {
        let found = self.resolve(scene_dir, Some(name));
        let stem = |p: &PathBuf| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_ascii_lowercase)
        };
        found.as_ref().and_then(stem)
            == Some(
                name.trim_end_matches(".ttf")
                    .trim_end_matches(".otf")
                    .to_ascii_lowercase(),
            )
    }

    pub(crate) fn font(
        &mut self,
        scene_dir: &Path,
        name: Option<&str>,
    ) -> Result<&FontVec, String> {
        let key = name.unwrap_or("").to_owned();
        if !self.loaded.contains_key(&key) {
            if let Some(name) = name
                && !self.exists(scene_dir, name)
            {
                eprintln!("enzwall: font {name:?} not found; using a fallback");
            }
            let path = self
                .resolve(scene_dir, name)
                .ok_or("No usable font found; set font = \"<file stem>\"")?;
            let data = crate::stage::read_asset(&path, 16 * 1024 * 1024, "Font exceeds 16 MiB")?;
            let font = FontVec::try_from_vec(data)
                .map_err(|_| format!("font {} is not a TrueType/OpenType font", path.display()))?;
            self.loaded.insert(key.clone(), font);
        }
        Ok(&self.loaded[&key])
    }
}

pub(crate) struct Coverage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Rasterises one line of text with `tracking` (em) between letters and
/// `pad` pixels of empty border for outline and shadow passes.
pub(crate) fn rasterize(
    font: &FontVec,
    text: &str,
    px: f32,
    tracking: f32,
    pad: f32,
) -> Result<Coverage, String> {
    let scaled = font.as_scaled(PxScale::from(px));
    let pad = (px * 0.1).ceil().max(2.0) + pad.ceil();
    let mut glyphs = Vec::new();
    let mut caret = pad;
    let mut previous = None;
    for c in text.chars().filter(|c| !c.is_control()) {
        let id = scaled.glyph_id(c);
        if let Some(previous) = previous {
            caret += scaled.kern(previous, id);
        }
        glyphs.push(id.with_scale_and_position(px, ab_glyph::point(caret, pad + scaled.ascent())));
        caret += scaled.h_advance(id) + tracking * px;
        previous = Some(id);
    }
    if !glyphs.is_empty() {
        caret -= tracking * px;
    }
    let width = (caret + pad).ceil().max(1.0) as u32;
    let height = (scaled.ascent() - scaled.descent() + pad * 2.0)
        .ceil()
        .max(1.0) as u32;
    if width > 8192 || height > 4096 || u64::from(width) * u64::from(height) > 4 * 1024 * 1024 {
        return Err("Text texture exceeds its raster budget".into());
    }
    let mut pixels = vec![0u8; (width * height) as usize];
    for glyph in glyphs {
        if let Some(outline) = scaled.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|x, y, coverage| {
                let (x, y) = (
                    x as i32 + bounds.min.x as i32,
                    y as i32 + bounds.min.y as i32,
                );
                if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
                    let at = (y as u32 * width + x as u32) as usize;
                    pixels[at] = pixels[at].max((coverage * 255.0).round() as u8);
                }
            });
        }
    }
    Ok(Coverage {
        width,
        height,
        pixels,
    })
}

pub(crate) fn dilate(source: &Coverage, radius: f32) -> Coverage {
    let (w, h) = (source.width as i32, source.height as i32);
    let r = radius.ceil() as i32;
    let offsets: Vec<(i32, i32, f32)> = (-r..=r)
        .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
        .filter_map(|(dx, dy)| {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            (d <= radius + 0.5).then_some((dx, dy, (radius + 0.5 - d).clamp(0.0, 1.0)))
        })
        .collect();
    let mut out = vec![0u8; source.pixels.len()];
    for y in 0..h {
        for x in 0..w {
            let mut best = 0.0f32;
            for &(dx, dy, weight) in &offsets {
                let (sx, sy) = (x + dx, y + dy);
                if sx >= 0 && sy >= 0 && sx < w && sy < h {
                    best = best.max(f32::from(source.pixels[(sy * w + sx) as usize]) * weight);
                    if best >= 255.0 {
                        break;
                    }
                }
            }
            out[(y * w + x) as usize] = best as u8;
        }
    }
    Coverage {
        width: source.width,
        height: source.height,
        pixels: out,
    }
}

/// Approximates a Gaussian blur with three box passes (soft shadows, glows).
pub(crate) fn blur(source: &Coverage, radius: f32) -> Coverage {
    let (w, h) = (source.width as usize, source.height as usize);
    let mut data: Vec<f32> = source.pixels.iter().map(|&v| f32::from(v)).collect();
    let box_radius = ((radius / 1.7).round() as usize).max(1);
    let mut scratch = vec![0.0f32; data.len()];
    for _ in 0..3 {
        for (line, stride, len, count) in [(w, 1usize, w, h), (1usize, w, h, w)] {
            for i in 0..count {
                let base = i * line;
                let mut sum = 0.0;
                let window = (2 * box_radius + 1) as f32;
                for k in 0..=box_radius.min(len - 1) {
                    sum += data[base + k * stride];
                }
                for k in 0..len {
                    scratch[base + k * stride] = sum / window;
                    if k + box_radius + 1 < len {
                        sum += data[base + (k + box_radius + 1) * stride];
                    }
                    if k >= box_radius {
                        sum -= data[base + (k - box_radius) * stride];
                    }
                }
            }
            std::mem::swap(&mut data, &mut scratch);
        }
    }
    Coverage {
        width: source.width,
        height: source.height,
        pixels: data
            .iter()
            .map(|v| v.round().clamp(0.0, 255.0) as u8)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_text_is_rejected_before_a_large_raster_allocation() {
        let font = FontVec::try_from_vec(
            include_bytes!("../../../scenes/_shared/fonts/Outfit-ExtraLight.ttf").to_vec(),
        )
        .expect("bundled Outfit font");
        let normal = rasterize(&font, "09:41", 96.0, 0.0, 2.0).expect("ordinary clock text");
        assert!(normal.pixels.iter().any(|p| *p > 0));
        assert!(rasterize(&font, &"W".repeat(256), 2048.0, 2.0, 1024.0).is_err());
    }
}
