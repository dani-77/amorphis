#![no_main]

use amorphis::image::{FitMode, ImageEffect, ImageLimits, RgbaImage};
use libfuzzer_sys::fuzz_target;
use std::num::NonZeroU32;

const LIMITS: ImageLimits = ImageLimits {
    max_width: 64,
    max_height: 64,
    max_pixels: 64 * 64,
    max_bytes: 64 * 64 * 4,
    max_blur_radius: 8,
    max_scratch_bytes: 64 * 64 * 8,
};

fuzz_target!(|data: &[u8]| {
    if data.len() < 6 {
        return;
    }

    let width = u32::from(data[0] % 64 + 1);
    let height = u32::from(data[1] % 64 + 1);
    let target_width = u32::from(data[2] % 64 + 1);
    let target_height = u32::from(data[3] % 64 + 1);
    let block_size = NonZeroU32::new(u32::from(data[4] % 64 + 1)).unwrap();
    let radius = u32::from(data[5] % 9);
    let payload = &data[6..];
    let pixel_len = width as usize * height as usize * 4;
    let mut pixels = vec![0_u8; pixel_len];
    if !payload.is_empty() {
        for (index, byte) in pixels.iter_mut().enumerate() {
            *byte = payload[index % payload.len()];
        }
    }

    let mut image = RgbaImage::from_rgba(width, height, pixels, LIMITS).unwrap();
    let effect = match data[0] % 4 {
        0 => ImageEffect::None,
        1 => ImageEffect::Blur { radius },
        2 => ImageEffect::Pixelate { block_size },
        _ => ImageEffect::BlurPixelate { radius, block_size },
    };
    effect.apply(&mut image, LIMITS).unwrap();

    let fit = match data[1] % 3 {
        0 => FitMode::Cover,
        1 => FitMode::Contain,
        _ => FitMode::Center,
    };
    let fitted = image
        .fitted(target_width, target_height, fit, [18, 18, 20, 255], LIMITS)
        .unwrap();
    assert_eq!(
        fitted.pixels().len(),
        target_width as usize * target_height as usize * 4
    );
});
