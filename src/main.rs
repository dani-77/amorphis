use amorphis::background::{BackgroundSource, StaticBackground};
use amorphis::image::{FitMode, ImageEffect, ImageLimits};
use amorphis::{backend, system};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const PAM_SERVICE: &str = "amorphis";
const PAM_SERVICE_FILE: &str = "/etc/pam.d/amorphis";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.as_slice() {
        [flag] if flag == "--version" || flag == "-V" => {
            println!("amorphis {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        [flag] if flag == "--help" || flag == "-h" => {
            println!(
                "Amorphis {}\n\nSecurity-first screen locker (early implementation).\n\nUsage:\n  amorphis --backend wayland|x11 [--image PATH.png | --solid]\n                                [--fit cover|contain|center]\n                                [--effect none|blur|pixelate|blur-pixelate]\n                                [--blur-radius 1..=128]\n                                [--pixel-size N]\n  amorphis --help\n  amorphis --version\n\nBoth backends use the embedded fallback image unless --image or --solid is selected.\nEffects apply to the selected PNG. Defaults: --effect none, --blur-radius 16,\n--pixel-size 16. Both backends require the PAM service {PAM_SERVICE_FILE}.\nTest development builds carefully before relying on them.",
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::SUCCESS
        }
        [option, backend, rest @ ..] if option == "--backend" && backend == "wayland" => {
            match parse_background(rest) {
                Ok(background) => run_wayland(background),
                Err(error) => {
                    eprintln!("amorphis: {error}; try --help (session unchanged)");
                    ExitCode::from(2)
                }
            }
        }
        [option, backend, rest @ ..] if option == "--backend" && backend == "x11" => {
            match parse_background(rest) {
                Ok(background) => run_x11(background),
                Err(error) => {
                    eprintln!("amorphis: {error}; try --help (session unchanged)");
                    ExitCode::from(2)
                }
            }
        }
        _ => {
            eprintln!("amorphis: invalid arguments; try --help (session unchanged)");
            ExitCode::from(2)
        }
    }
}

fn run_x11(background: Option<StaticBackground>) -> ExitCode {
    let Some(username) = lock_identity() else {
        return ExitCode::from(2);
    };
    match backend::x11::run(PAM_SERVICE, &username, background) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("amorphis: {error}");
            ExitCode::from(1)
        }
    }
}

fn run_wayland(background: Option<StaticBackground>) -> ExitCode {
    let Some(username) = lock_identity() else {
        return ExitCode::from(2);
    };

    match backend::wayland::run(PAM_SERVICE, &username, background) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("amorphis: {error}");
            ExitCode::from(1)
        }
    }
}

fn lock_identity() -> Option<String> {
    // Avoid entering a session lock that can never authenticate merely because
    // the distribution-specific PAM policy has not yet been installed.
    if !Path::new(PAM_SERVICE_FILE).is_file() {
        eprintln!("amorphis: refusing to lock: PAM service {PAM_SERVICE_FILE} is not installed");
        return None;
    }

    if let Err(error) = system::harden_process() {
        eprintln!("amorphis: refusing to lock: {error}");
        return None;
    }

    match system::current_username() {
        Ok(username) => Some(username),
        Err(error) => {
            eprintln!("amorphis: refusing to lock: {error}");
            None
        }
    }
}

fn parse_background(arguments: &[String]) -> Result<Option<StaticBackground>, &'static str> {
    let mut image: Option<PathBuf> = None;
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
            || fit_was_set
            || effect_name.is_some()
            || blur_radius.is_some()
            || pixel_size.is_some()
        {
            return Err("--solid cannot be combined with image options");
        }
        return Ok(None);
    }

    let source = image.map_or(BackgroundSource::EmbeddedFallback, BackgroundSource::File);
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
}
