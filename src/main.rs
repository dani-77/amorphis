use amorphis::background::StaticBackground;
use amorphis::cli::parse_background;
use amorphis::{backend, system};
use std::path::Path;
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
                "Amorphis {}\n\nSecurity-first screen locker (early implementation).\n\nUsage:\n  amorphis --backend wayland|x11 [--image PATH.png | --capture | --solid]\n                                [--fit cover|contain|center]\n                                [--effect none|blur|pixelate|blur-pixelate]\n                                [--blur-radius 1..=128]\n                                [--pixel-size N]\n  amorphis --help\n  amorphis --version\n\nBoth backends use the embedded fallback image unless --image or --solid is selected.\nBoth also support --capture when the display server exposes the required capture\nmechanism. Effects apply to the selected background. Defaults: --effect none,\n--blur-radius 16, --pixel-size 16. Both backends require the PAM service\n{PAM_SERVICE_FILE}. Test development builds carefully before relying on them.",
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
