//! Easing curves and small animation helpers.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    EaseOutBack,
    EaseOutExpo,
}

impl Easing {
    pub fn parse(s: &str) -> Result<Self, String> {
        Ok(match s {
            "linear" => Self::Linear,
            "ease_in" => Self::EaseIn,
            "ease_out" => Self::EaseOut,
            "ease_in_out" => Self::EaseInOut,
            "ease_out_back" | "back" => Self::EaseOutBack,
            "ease_out_expo" | "expo" => Self::EaseOutExpo,
            _ => {
                return Err(format!(
                    "unknown easing '{s}' (linear|ease_in|ease_out|ease_in_out|ease_out_back|ease_out_expo)"
                ));
            }
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::EaseIn => "ease_in",
            Self::EaseOut => "ease_out",
            Self::EaseInOut => "ease_in_out",
            Self::EaseOutBack => "ease_out_back",
            Self::EaseOutExpo => "ease_out_expo",
        }
    }

    /// Map linear progress `t` in 0..=1 to eased progress.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t * t,
            Self::EaseOut => 1.0 - (1.0 - t).powi(3),
            Self::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Self::EaseOutBack => {
                let (c1, c3) = (1.70158, 2.70158);
                1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
            }
            Self::EaseOutExpo => {
                if t >= 1.0 {
                    1.0
                } else {
                    1.0 - 2f32.powf(-10.0 * t)
                }
            }
        }
    }
}

/// Frame-rate independent exponential approach of `current` to `target`.
/// `duration` is roughly the time to get ~98% of the way; 0 snaps.
/// Returns true while still moving.
pub fn approach(current: &mut f64, target: f64, duration: f32, dt: f32, epsilon: f64) -> bool {
    if duration <= 0.0 || (*current - target).abs() <= epsilon {
        *current = target;
        return false;
    }
    let k = 1.0 - (-(dt as f64) * 4.0 / duration as f64).exp();
    *current += (target - *current) * k;
    if (*current - target).abs() <= epsilon {
        *current = target;
        return false;
    }
    true
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints() {
        for e in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::EaseOutBack,
            Easing::EaseOutExpo,
        ] {
            assert!(e.apply(0.0).abs() < 1e-4, "{e:?}");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-4, "{e:?}");
            assert_eq!(Easing::parse(e.name()).unwrap(), e);
        }
    }

    #[test]
    fn approach_converges() {
        let mut v = 0.0;
        let mut frames = 0;
        while approach(&mut v, 1.0, 0.2, 1.0 / 60.0, 1e-3) {
            frames += 1;
            assert!(frames < 120);
        }
        assert_eq!(v, 1.0);
        assert!(
            !approach(&mut v, 5.0, 0.0, 0.016, 1e-3) && v == 5.0,
            "zero duration snaps"
        );
    }
}
