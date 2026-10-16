//! Backend-independent authentication feedback drawn over the lock frame.

use crate::image::RgbaImage;
use std::time::{Duration, Instant};

const INPUT_VISIBLE_FOR: Duration = Duration::from_millis(350);
const FAILURE_VISIBLE_FOR: Duration = Duration::from_millis(1500);
const VERIFY_FRAME_TIME: Duration = Duration::from_millis(250);
const TEAL: [u8; 4] = [45, 212, 191, 255];
const RED: [u8; 4] = [239, 68, 68, 255];
const DARK: [u8; 4] = [8, 12, 15, 255];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndicatorStyle {
    Hidden,
    Input,
    Verifying(u8),
    Failed,
}

#[derive(Clone, Copy, Debug)]
enum IndicatorState {
    Hidden,
    Input { until: Instant },
    Verifying { since: Instant },
    Failed { until: Instant },
}

/// Tracks only presentation state. It has no authority over authentication or
/// the lock lifecycle.
pub struct AuthIndicator {
    state: IndicatorState,
    rendered: IndicatorStyle,
}

impl Default for AuthIndicator {
    fn default() -> Self {
        Self {
            state: IndicatorState::Hidden,
            rendered: IndicatorStyle::Hidden,
        }
    }
}

impl AuthIndicator {
    pub fn input(&mut self) {
        self.input_at(Instant::now());
    }

    pub fn verifying(&mut self) {
        self.state = IndicatorState::Verifying {
            since: Instant::now(),
        };
    }

    pub fn failed(&mut self) {
        self.state = IndicatorState::Failed {
            until: Instant::now() + FAILURE_VISIBLE_FOR,
        };
    }

    #[must_use]
    pub fn style(&self) -> IndicatorStyle {
        self.style_at(Instant::now())
    }

    /// Return the new style only when a redraw is needed.
    pub fn take_redraw(&mut self) -> Option<IndicatorStyle> {
        let style = self.style();
        if style == self.rendered {
            None
        } else {
            self.rendered = style;
            Some(style)
        }
    }

    fn input_at(&mut self, now: Instant) {
        // Keep a recent failure visible even if the user immediately begins
        // preparing another attempt. Submitting that attempt may still move
        // the indicator to the verifying state.
        if matches!(self.state, IndicatorState::Failed { until } if now < until) {
            return;
        }
        self.state = IndicatorState::Input {
            until: now + INPUT_VISIBLE_FOR,
        };
    }

    fn style_at(&self, now: Instant) -> IndicatorStyle {
        match self.state {
            IndicatorState::Hidden => IndicatorStyle::Hidden,
            IndicatorState::Input { until } if now < until => IndicatorStyle::Input,
            IndicatorState::Input { .. } => IndicatorStyle::Hidden,
            IndicatorState::Verifying { since } => {
                let frame =
                    (now.duration_since(since).as_millis() / VERIFY_FRAME_TIME.as_millis()) % 3;
                IndicatorStyle::Verifying(frame as u8)
            }
            IndicatorState::Failed { until } if now < until => IndicatorStyle::Failed,
            IndicatorState::Failed { .. } => IndicatorStyle::Hidden,
        }
    }
}

/// Draw a compact central ring. The three verifying dots are animation only;
/// they never encode the secret length.
pub fn draw_indicator(image: &mut RgbaImage, style: IndicatorStyle) {
    if style == IndicatorStyle::Hidden {
        return;
    }
    let width = image.width();
    let height = image.height();
    let shortest = width.min(height);
    if shortest < 32 {
        return;
    }

    let radius = (shortest / 18).clamp(30, 52).min(shortest / 3) as i64;
    let ring = (radius / 7).clamp(4, 8);
    let shadow = 3_i64;
    let center_x = i64::from(width / 2);
    let center_y = i64::from(height / 2);
    let extent = radius + shadow;
    let left = (center_x - extent).max(0) as u32;
    let right = (center_x + extent).min(i64::from(width) - 1) as u32;
    let top = (center_y - extent).max(0) as u32;
    let bottom = (center_y + extent).min(i64::from(height) - 1) as u32;
    let inner = radius - ring;
    let outer_squared = radius * radius;
    let inner_squared = inner * inner;
    let shadow_squared = (radius + shadow) * (radius + shadow);
    let colour = if style == IndicatorStyle::Failed {
        RED
    } else {
        TEAL
    };

    for y in top..=bottom {
        for x in left..=right {
            let dx = i64::from(x) - center_x;
            let dy = i64::from(y) - center_y;
            let distance = dx * dx + dy * dy;
            let overlay = if distance <= inner_squared {
                Some((DARK, 176))
            } else if distance <= outer_squared {
                Some((colour, 255))
            } else if distance <= shadow_squared {
                Some((DARK, 150))
            } else {
                None
            };
            if let Some((overlay, alpha)) = overlay {
                blend_pixel(image, x, y, overlay, alpha);
            }
        }
    }

    match style {
        IndicatorStyle::Verifying(active) => {
            draw_verifying_dots(image, center_x, center_y, radius, active)
        }
        IndicatorStyle::Failed => draw_failure_cross(image, center_x, center_y, radius),
        IndicatorStyle::Hidden | IndicatorStyle::Input => {}
    }
}

fn draw_verifying_dots(
    image: &mut RgbaImage,
    center_x: i64,
    center_y: i64,
    radius: i64,
    active: u8,
) {
    let dot_radius = (radius / 10).max(3);
    let spacing = radius / 3;
    for (index, offset) in [-spacing, 0, spacing].into_iter().enumerate() {
        let alpha = if index == usize::from(active) {
            255
        } else {
            90
        };
        draw_disc(image, center_x + offset, center_y, dot_radius, TEAL, alpha);
    }
}

fn draw_failure_cross(image: &mut RgbaImage, center_x: i64, center_y: i64, radius: i64) {
    let arm = radius / 3;
    let thickness = (radius / 12).max(3);
    for dy in -arm..=arm {
        for dx in -arm..=arm {
            if (dx - dy).abs() <= thickness || (dx + dy).abs() <= thickness {
                set_pixel(image, center_x + dx, center_y + dy, RED, 255);
            }
        }
    }
}

fn draw_disc(
    image: &mut RgbaImage,
    center_x: i64,
    center_y: i64,
    radius: i64,
    colour: [u8; 4],
    alpha: u8,
) {
    let squared = radius * radius;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy <= squared {
                set_pixel(image, center_x + dx, center_y + dy, colour, alpha);
            }
        }
    }
}

fn set_pixel(image: &mut RgbaImage, x: i64, y: i64, colour: [u8; 4], alpha: u8) {
    if x < 0 || y < 0 || x >= i64::from(image.width()) || y >= i64::from(image.height()) {
        return;
    }
    blend_pixel(image, x as u32, y as u32, colour, alpha);
}

fn blend_pixel(image: &mut RgbaImage, x: u32, y: u32, colour: [u8; 4], alpha: u8) {
    let offset = (y as usize * image.width() as usize + x as usize) * 4;
    let target = &mut image.pixels_mut()[offset..offset + 4];
    let alpha = u32::from(alpha);
    let inverse = 255 - alpha;
    for channel in 0..3 {
        target[channel] =
            ((u32::from(colour[channel]) * alpha + u32::from(target[channel]) * inverse + 127)
                / 255) as u8;
    }
    target[3] = 255;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::ImageLimits;

    #[test]
    fn input_and_failure_expire_without_encoding_secret_length() {
        let start = Instant::now();
        let mut indicator = AuthIndicator::default();
        indicator.input_at(start);
        assert_eq!(indicator.style_at(start), IndicatorStyle::Input);
        assert_eq!(
            indicator.style_at(start + INPUT_VISIBLE_FOR),
            IndicatorStyle::Hidden
        );

        indicator.state = IndicatorState::Failed {
            until: start + FAILURE_VISIBLE_FOR,
        };
        indicator.input_at(start + Duration::from_millis(10));
        assert_eq!(
            indicator.style_at(start + Duration::from_millis(10)),
            IndicatorStyle::Failed
        );
    }

    #[test]
    fn verifying_cycles_three_fixed_frames() {
        let start = Instant::now();
        let indicator = AuthIndicator {
            state: IndicatorState::Verifying { since: start },
            rendered: IndicatorStyle::Hidden,
        };
        assert_eq!(indicator.style_at(start), IndicatorStyle::Verifying(0));
        assert_eq!(
            indicator.style_at(start + VERIFY_FRAME_TIME),
            IndicatorStyle::Verifying(1)
        );
        assert_eq!(
            indicator.style_at(start + VERIFY_FRAME_TIME * 3),
            IndicatorStyle::Verifying(0)
        );
    }

    #[test]
    fn renderer_changes_centre_but_not_image_edges() {
        let limits = ImageLimits::default();
        let mut image = RgbaImage::solid(320, 200, [20, 30, 40, 255], limits).unwrap();
        let edge = image.pixels()[0..4].to_vec();
        let center = ((100 * 320 + 160) * 4) as usize;
        let before = image.pixels()[center..center + 4].to_vec();

        draw_indicator(&mut image, IndicatorStyle::Failed);

        assert_eq!(&image.pixels()[0..4], edge.as_slice());
        assert_ne!(&image.pixels()[center..center + 4], before.as_slice());
    }
}
