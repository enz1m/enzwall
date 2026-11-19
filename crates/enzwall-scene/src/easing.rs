#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    Linear,
    InQuad,
    OutQuad,
    InOutQuad,
    InCubic,
    OutCubic,
    InOutCubic,
    InOutSine,
    OutExpo,
    OutBack,
}

impl Easing {
    pub fn parse(name: &str) -> Result<Self, String> {
        Ok(match name {
            "linear" => Self::Linear,
            "in-quad" => Self::InQuad,
            "out-quad" => Self::OutQuad,
            "in-out-quad" => Self::InOutQuad,
            "in-cubic" => Self::InCubic,
            "out-cubic" => Self::OutCubic,
            "in-out-cubic" => Self::InOutCubic,
            "in-out-sine" => Self::InOutSine,
            "out-expo" => Self::OutExpo,
            "out-back" => Self::OutBack,
            other => return Err(format!("unknown easing {other:?}")),
        })
    }

    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::InQuad => t * t,
            Self::OutQuad => 1.0 - (1.0 - t) * (1.0 - t),
            Self::InOutQuad => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
            Self::InCubic => t * t * t,
            Self::OutCubic => 1.0 - (1.0 - t).powi(3),
            Self::InOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Self::InOutSine => -((std::f32::consts::PI * t).cos() - 1.0) / 2.0,
            Self::OutExpo => {
                if t >= 1.0 {
                    1.0
                } else {
                    1.0 - 2f32.powf(-10.0 * t)
                }
            }
            Self::OutBack => {
                let c1 = 1.70158;
                let c3 = c1 + 1.0;
                1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
            }
        }
    }
}
