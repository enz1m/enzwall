use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SceneFile {
    pub format: u32,
    pub name: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub fps: Option<u32>,
    pub transition_fps: Option<u32>,
    pub background: Option<String>,
    pub render_scale: Option<f32>,
    pub idle_timeout: Option<u32>,
    pub sleep_dim: Option<f32>,
    pub parallax: Option<ParallaxFile>,
    #[serde(rename = "layer", default)]
    pub layers: Vec<LayerFile>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParallaxFile {
    pub shift: Option<f32>,
    pub duration: Option<f32>,
    pub easing: Option<String>,
    pub range: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LayerFile {
    pub role: Option<String>,
    pub kind: String,
    pub id: Option<String>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub w: Option<f32>,
    pub h: Option<f32>,
    pub radius: Option<f32>,
    pub rotation: Option<f32>,
    pub scale: Option<f32>,
    pub opacity: Option<f32>,
    pub phase: Option<f32>,
    pub depth: Option<f32>,
    pub blend: Option<String>,
    pub color: Option<String>,
    pub colors: Option<Vec<String>>,
    pub angle: Option<f32>,
    pub radial: Option<bool>,
    pub softness: Option<f32>,
    pub thickness: Option<f32>,
    pub corner: Option<f32>,
    pub caps: Option<String>,
    pub shade: Option<String>,
    pub repeat: Option<[f32; 2]>,
    pub tracking: Option<f32>,
    pub case: Option<String>,
    pub outline: Option<String>,
    pub outline_width: Option<f32>,
    pub shadow: Option<String>,
    pub shadow_offset: Option<[f32; 2]>,
    pub shadow_blur: Option<f32>,
    pub count: Option<u32>,
    pub area: Option<[f32; 4]>,
    pub sizes: Option<[f32; 2]>,
    pub velocity: Option<[f32; 2]>,
    pub jitter: Option<f32>,
    pub wobble: Option<f32>,
    pub wobble_period: Option<f32>,
    pub twinkle: Option<f32>,
    pub seed: Option<u32>,
    pub spin: Option<f32>,
    pub density: Option<f32>,
    pub columns: Option<f32>,
    pub fade: Option<f32>,
    pub glow: Option<f32>,
    pub pivot: Option<[f32; 2]>,
    pub stretch: Option<f32>,
    pub sweep: Option<f32>,
    pub start: Option<f32>,
    pub bind: Option<String>,
    pub track: Option<String>,
    pub amplitude: Option<f32>,
    pub frequency: Option<f32>,
    pub detail: Option<f32>,
    pub fill: Option<String>,
    pub source: Option<String>,
    pub fit: Option<String>,
    pub tile: Option<bool>,
    pub text: Option<String>,
    pub font: Option<String>,
    pub size: Option<f32>,
    pub align: Option<String>,
    #[serde(default)]
    pub motion: Vec<MotionFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MotionFile {
    pub property: String,
    pub on: Option<String>,
    pub wave: Option<String>,
    pub period: Option<f32>,
    pub amplitude: Option<f32>,
    pub phase: Option<f32>,
    pub from: Option<f32>,
    pub to: Option<f32>,
    pub duration: Option<f32>,
    pub delay: Option<f32>,
    pub easing: Option<String>,
    pub hold: Option<bool>,
    pub unit: Option<String>,
}
