use amorphis::image::FitMode;
use amorphis::{backend, system};
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
                "Amorphis {}\n\nSecurity-first screen locker (early implementation).\n\nUsage:\n  amorphis --backend wayland [--image PATH.png]\n                            [--fit cover|contain|center]\n  amorphis --backend x11\n  amorphis --help\n  amorphis --version\n\nBoth backends require the PAM service {PAM_SERVICE_FILE}.\nTest development builds in a nested compositor or X server first.",
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
        [option, backend] if option == "--backend" && backend == "x11" => run_x11(),
        _ => {
            eprintln!("amorphis: invalid arguments; try --help (session unchanged)");
            ExitCode::from(2)
        }
    }
}

fn run_x11() -> ExitCode {
    let Some(username) = lock_identity() else {
        return ExitCode::from(2);
    };
    match backend::x11::run(PAM_SERVICE, &username) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("amorphis: {error}");
            ExitCode::from(1)
        }
    }
}

fn run_wayland(background: Option<backend::wayland::StaticBackground>) -> ExitCode {
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

fn parse_background(
    arguments: &[String],
) -> Result<Option<backend::wayland::StaticBackground>, &'static str> {
    let mut image: Option<PathBuf> = None;
    let mut fit = FitMode::Cover;
    let mut fit_was_set = false;
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
            "--fit" => return Err("--fit may only be specified once"),
            _ => return Err("unknown Wayland option"),
        }
    }

    match image {
        Some(path) => Ok(Some(backend::wayland::StaticBackground { path, fit })),
        None if fit_was_set => Err("--fit requires --image"),
        None => Ok(None),
    }
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
        assert_eq!(parsed.path, PathBuf::from("wall.png"));
        assert_eq!(parsed.fit, FitMode::Contain);
    }

    #[test]
    fn fit_without_image_is_rejected() {
        assert!(parse_background(&arguments(&["--fit", "center"])).is_err());
    }
}
