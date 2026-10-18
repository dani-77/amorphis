#![no_main]

use amorphis::image::{ImageLimits, load_png_bytes};
use libfuzzer_sys::fuzz_target;

const LIMITS: ImageLimits = ImageLimits {
    max_width: 256,
    max_height: 256,
    max_pixels: 256 * 256,
    max_bytes: 256 * 256 * 4,
    max_blur_radius: 8,
    max_scratch_bytes: 256 * 256 * 8,
};

// A valid 1x1 PNG gives libFuzzer a compact path into the successful decoder
// states without committing a separate binary corpus.
const VALID_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0,
    0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15, 0, 1, 5, 1, 1,
    39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

fuzz_target!(|data: &[u8]| {
    assert!(load_png_bytes(VALID_PNG, LIMITS).is_ok());

    let _ = load_png_bytes(data, LIMITS);

    let mut near_valid = VALID_PNG.to_vec();
    for (index, byte) in data.iter().take(32).enumerate() {
        let offset = (index * 17 + usize::from(*byte)) % near_valid.len();
        near_valid[offset] ^= *byte;
    }
    let _ = load_png_bytes(&near_valid, LIMITS);
});
