//! Bounded command-line parsing shared by the binary and fuzz targets.

use crate::background::{BackgroundSource, StaticBackground};
use crate::image::{FitMode, ImageEffect, ImageLimits};
use std::num::NonZeroU32;
use std::path::PathBuf;

pub fn parse_background(arguments: &[String]) -> Result<Option<StaticBackground>, &'static str> {
    let mut image: Option<PathBuf> = None;
    let mut capture = false;
    let mut solid = false;
    let mut fit = FitMode::Cover;
    let mut fit_was_set = false;
    let mut effect_name: Option<&str> = None;
    let mut blur_radius: Option<u32> = None;
    let mut pixel_size: Option<NonZeroU32> = None;
    let mut index = 0;

    while index < arguments.len() {
        match arguments[index].as_str() {
            "--image" if image.is_none() => {
                let value = arguments
                    .get(index + 1)
                    .ok_or("--image requires a PNG path")?;
                if value.is_empty() {
                    return Err("--image requires a non-empty PNG path");
                }
                image = Some(PathBuf::from(value));
                index += 2;
            }
            "--fit" if !fit_was_set => {
                let value = arguments
                    .get(index + 1)
                    .ok_or("--fit requires cover, contain, or center")?;
                fit = match value.as_str() {
                    "cover" => FitMode::Cover,
                    "contain" => FitMode::Contain,
                    "center" => FitMode::Center,
                    _ => return Err("--fit requires cover, contain, or center"),
                };
                fit_was_set = true;
                index += 2;
            }
            "--image" => return Err("--image may only be specified once"),
            "--capture" if !capture => {
                capture = true;
                index += 1;
            }
            "--capture" => return Err("--capture may only be specified once"),
            "--solid" if !solid => {
                solid = true;
                index += 1;
            }
            "--solid" => return Err("--solid may only be specified once"),
            "--fit" => return Err("--fit may only be specified once"),
            "--effect" if effect_name.is_none() => {
                let value = arguments
                    .get(index + 1)
                    .ok_or("--effect requires none, blur, pixelate, or blur-pixelate")?;
                match value.as_str() {
                    "none" | "blur" | "pixelate" | "blur-pixelate" => {
                        effect_name = Some(value);
                    }
                    _ => {
                        return Err("--effect requires none, blur, pixelate, or blur-pixelate");
                    }
                }
                index += 2;
            }
            "--blur-radius" if blur_radius.is_none() => {
                let value = arguments
                    .get(index + 1)
                    .ok_or("--blur-radius requires an integer from 1 to 128")?;
                let parsed = value
                    .parse::<u32>()
                    .map_err(|_| "--blur-radius requires an integer from 1 to 128")?;
                if parsed == 0 || parsed > ImageLimits::default().max_blur_radius {
                    return Err("--blur-radius requires an integer from 1 to 128");
                }
                blur_radius = Some(parsed);
                index += 2;
            }
            "--pixel-size" if pixel_size.is_none() => {
                let value = arguments
                    .get(index + 1)
                    .ok_or("--pixel-size requires a positive integer")?;
                let parsed = value
                    .parse::<u32>()
                    .ok()
                    .and_then(NonZeroU32::new)
                    .ok_or("--pixel-size requires a positive integer")?;
                pixel_size = Some(parsed);
                index += 2;
            }
            "--effect" => return Err("--effect may only be specified once"),
            "--blur-radius" => return Err("--blur-radius may only be specified once"),
            "--pixel-size" => return Err("--pixel-size may only be specified once"),
            _ => return Err("unknown background option"),
        }
    }

    let effect = match effect_name.unwrap_or("none") {
        "none" if blur_radius.is_some() || pixel_size.is_some() => {
            return Err("effect parameters require a matching --effect");
        }
        "none" => ImageEffect::None,
        "blur" if pixel_size.is_some() => return Err("--pixel-size requires a pixelate effect"),
        "blur" => ImageEffect::Blur {
            radius: blur_radius.unwrap_or(16),
        },
        "pixelate" if blur_radius.is_some() => return Err("--blur-radius requires a blur effect"),
        "pixelate" => ImageEffect::Pixelate {
            block_size: pixel_size.unwrap_or(NonZeroU32::new(16).unwrap()),
        },
        "blur-pixelate" => ImageEffect::BlurPixelate {
            radius: blur_radius.unwrap_or(16),
            block_size: pixel_size.unwrap_or(NonZeroU32::new(16).unwrap()),
        },
        _ => unreachable!("effect names are validated while parsing"),
    };

    if solid {
        if image.is_some()
            || capture
            || fit_was_set
            || effect_name.is_some()
            || blur_radius.is_some()
            || pixel_size.is_some()
        {
            return Err("--solid cannot be combined with background options");
        }
        return Ok(None);
    }

    if capture && image.is_some() {
        return Err("--capture cannot be combined with --image");
    }

    let source = if capture {
        BackgroundSource::DesktopCapture
    } else {
        image.map_or(BackgroundSource::EmbeddedFallback, BackgroundSource::File)
    };
    Ok(Some(StaticBackground {
        source,
        fit,
        effect,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parses_static_png_options() {
        let parsed = parse_background(&arguments(&["--image", "wall.png", "--fit", "contain"]))
            .unwrap()
            .unwrap();
        assert_eq!(
            parsed.source,
            BackgroundSource::File(PathBuf::from("wall.png"))
        );
        assert_eq!(parsed.fit, FitMode::Contain);
        assert_eq!(parsed.effect, ImageEffect::None);
    }

    #[test]
    fn default_and_fit_without_path_use_embedded_fallback() {
        let default = parse_background(&[]).unwrap().unwrap();
        assert_eq!(default.source, BackgroundSource::EmbeddedFallback);

        let centered = parse_background(&arguments(&["--fit", "center"]))
            .unwrap()
            .unwrap();
        assert_eq!(centered.source, BackgroundSource::EmbeddedFallback);
        assert_eq!(centered.fit, FitMode::Center);
    }

    #[test]
    fn parses_combined_effect_with_explicit_parameters() {
        let parsed = parse_background(&arguments(&[
            "--image",
            "wall.png",
            "--effect",
            "blur-pixelate",
            "--blur-radius",
            "24",
            "--pixel-size",
            "12",
        ]))
        .unwrap()
        .unwrap();

        assert_eq!(
            parsed.effect,
            ImageEffect::BlurPixelate {
                radius: 24,
                block_size: NonZeroU32::new(12).unwrap(),
            }
        );
    }

    #[test]
    fn parses_capture_with_effects_and_rejects_conflicting_sources() {
        let parsed = parse_background(&arguments(&["--capture", "--effect", "blur-pixelate"]))
            .unwrap()
            .unwrap();
        assert_eq!(parsed.source, BackgroundSource::DesktopCapture);
        assert_eq!(
            parsed.effect,
            ImageEffect::BlurPixelate {
                radius: 16,
                block_size: NonZeroU32::new(16).unwrap(),
            }
        );

        assert!(parse_background(&arguments(&["--capture", "--image", "wall.png"])).is_err());
        assert!(parse_background(&arguments(&["--capture", "--solid"])).is_err());
    }

    #[test]
    fn rejects_effect_parameters_that_would_be_ignored() {
        assert!(
            parse_background(&arguments(&[
                "--image",
                "wall.png",
                "--effect",
                "pixelate",
                "--blur-radius",
                "4",
            ]))
            .is_err()
        );
        let fallback_effect = parse_background(&arguments(&["--effect", "blur"]))
            .unwrap()
            .unwrap();
        assert_eq!(fallback_effect.effect, ImageEffect::Blur { radius: 16 });
        assert!(
            parse_background(&arguments(&[
                "--image",
                "wall.png",
                "--effect",
                "blur",
                "--blur-radius",
                "129",
            ]))
            .is_err()
        );
    }

    #[test]
    fn solid_background_rejects_image_options() {
        assert!(
            parse_background(&arguments(&["--solid"]))
                .unwrap()
                .is_none()
        );
        assert!(parse_background(&arguments(&["--solid", "--effect", "blur"])).is_err());
        assert!(parse_background(&arguments(&["--solid", "--image", "wall.png"])).is_err());
    }

    #[test]
    fn rejects_empty_huge_and_duplicate_values_without_panicking() {
        for arguments in [
            arguments(&["--image", ""]),
            arguments(&["--pixel-size", "0", "--effect", "pixelate"]),
            arguments(&["--pixel-size", "4294967296", "--effect", "pixelate"]),
            arguments(&["--blur-radius", "999999999999999999999999"]),
            arguments(&["--fit", "cover", "--fit", "center"]),
            arguments(&["--effect", "blur", "--effect", "none"]),
        ] {
            assert!(parse_background(&arguments).is_err());
        }
    }
}
