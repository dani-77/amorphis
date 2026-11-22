//! Per-session background profiles. Parse every invocation, before entering the lock.

use crate::background::StaticBackground;
use crate::cli::parse_background;
use crate::session::Session;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    image: Option<String>,
    capture: Option<bool>,
    solid: Option<bool>,
    fit: Option<String>,
    effect: Option<String>,
    blur_radius: Option<u32>,
    pixel_size: Option<u32>,
}

pub fn load(dir: &Path, session: Session) -> Result<Option<StaticBackground>, String> {
    let path = dir.join(format!("{}.toml", session.name()));
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return parse_background(&[]).map_err(str::to_owned);
        }
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    parse(&source).map_err(|error| format!("{}: invalid profile: {error}", path.display()))
}

fn parse(source: &str) -> Result<Option<StaticBackground>, String> {
    let profile: Profile = toml::from_str(source).map_err(|error| error.to_string())?;
    let mut args = Vec::new();
    if let Some(value) = profile.image {
        args.extend(["--image".to_owned(), value]);
    }
    if profile.capture == Some(true) {
        args.push("--capture".to_owned());
    }
    if profile.solid == Some(true) {
        args.push("--solid".to_owned());
    }
    if let Some(value) = profile.fit {
        args.extend(["--fit".to_owned(), value]);
    }
    if let Some(value) = profile.effect {
        args.extend(["--effect".to_owned(), value]);
    }
    if let Some(value) = profile.blur_radius {
        args.extend(["--blur-radius".to_owned(), value.to_string()]);
    }
    if let Some(value) = profile.pixel_size {
        args.extend(["--pixel-size".to_owned(), value.to_string()]);
    }
    parse_background(&args).map_err(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::BackgroundSource;

    #[test]
    fn parses_profile_with_cli_validation() {
        let result = parse("capture = true\neffect = 'blur'\nblur_radius = 10")
            .unwrap()
            .unwrap();
        assert_eq!(result.source, BackgroundSource::DesktopCapture);
        assert!(parse("solid = true\nimage = 'wall.png'").is_err());
        assert!(parse("effect = 'blur'\npixel_size = 5").is_err());
        assert!(parse("typo = 1").is_err());
    }
}
