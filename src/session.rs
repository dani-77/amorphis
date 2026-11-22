//! Session detection shared by the locker and idle daemon.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Session {
    X11,
    Wayland,
}

impl Session {
    pub fn detect() -> Result<Self, &'static str> {
        detect(
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            std::env::var("DISPLAY").ok().as_deref(),
        )
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::X11 => "x11",
            Self::Wayland => "wayland",
        }
    }
}

fn detect(
    kind: Option<&str>,
    wayland: Option<&str>,
    x11: Option<&str>,
) -> Result<Session, &'static str> {
    match kind {
        Some("wayland") => return Ok(Session::Wayland),
        Some("x11") if wayland.filter(|s| !s.is_empty()).is_none() => return Ok(Session::X11),
        _ => {}
    }
    if wayland.is_some_and(|s| !s.is_empty()) {
        Ok(Session::Wayland)
    } else if x11.is_some_and(|s| !s.is_empty()) {
        Ok(Session::X11)
    } else {
        Err(
            "cannot detect X11 or Wayland session (set XDG_SESSION_TYPE, WAYLAND_DISPLAY, or DISPLAY)",
        )
    }
}

pub fn config_dir() -> Result<std::path::PathBuf, &'static str> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|v| std::path::PathBuf::from(v).join(".config").into_os_string())
        })
        .ok_or("XDG_CONFIG_HOME and HOME are unset")?;
    Ok(std::path::PathBuf::from(base).join("amorphis"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wayland_wins_over_xwayland_display() {
        assert_eq!(
            detect(Some("wayland"), Some("wayland-0"), Some(":0")),
            Ok(Session::Wayland)
        );
        assert_eq!(
            detect(None, Some("wayland-0"), Some(":0")),
            Ok(Session::Wayland)
        );
    }
    #[test]
    fn detects_x11_and_rejects_missing_session() {
        assert_eq!(detect(Some("x11"), None, Some(":0")), Ok(Session::X11));
        assert!(detect(None, None, None).is_err());
    }
}
