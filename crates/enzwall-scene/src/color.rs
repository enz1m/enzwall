/// Straight (non-premultiplied) sRGB colour with components in 0–1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub fn parse(text: &str) -> Result<Self, String> {
        let hex = text
            .strip_prefix('#')
            .filter(|h| (h.len() == 6 || h.len() == 8) && h.bytes().all(|b| b.is_ascii_hexdigit()));
        let hex =
            hex.ok_or_else(|| format!("invalid colour {text:?} (use #rrggbb or #rrggbbaa)"))?;
        let channel = |at: usize| -> Result<f32, String> {
            let pair = hex.get(at..at + 2).ok_or("Invalid colour channel")?;
            u8::from_str_radix(pair, 16)
                .map(|value| f32::from(value) / 255.0)
                .map_err(|error| error.to_string())
        };
        Ok(Self {
            r: channel(0)?,
            g: channel(2)?,
            b: channel(4)?,
            a: if hex.len() == 8 { channel(6)? } else { 1.0 },
        })
    }
}
