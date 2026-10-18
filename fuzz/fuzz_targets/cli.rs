#![no_main]

use amorphis::cli::parse_background;
use libfuzzer_sys::fuzz_target;

const TOKENS: &[&str] = &[
    "",
    "--image",
    "wall.png",
    "--capture",
    "--solid",
    "--fit",
    "cover",
    "contain",
    "center",
    "--effect",
    "none",
    "blur",
    "pixelate",
    "blur-pixelate",
    "--blur-radius",
    "0",
    "1",
    "128",
    "129",
    "4294967296",
    "--pixel-size",
    "16",
    "unknown",
];

fuzz_target!(|data: &[u8]| {
    let arguments: Vec<String> = data
        .iter()
        .take(64)
        .map(|byte| TOKENS[usize::from(*byte) % TOKENS.len()].to_owned())
        .collect();
    let _ = parse_background(&arguments);
});
