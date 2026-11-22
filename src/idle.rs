//! Idle rules and command execution, independent of display protocols.

use crate::session::Session;
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub timeout: u32,
    pub command: Option<String>,
    pub command_x11: Option<String>,
    pub command_wayland: Option<String>,
    pub resume_command: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    rule: Vec<Rule>,
}

#[derive(Debug)]
pub struct ActiveRule {
    pub timeout_ms: u32,
    command: String,
    resume_command: Option<String>,
    fired: bool,
}

impl ActiveRule {
    pub fn idled(&mut self) {
        if let Some(command) = self.transition(true) {
            spawn(command);
        }
    }

    pub fn resumed(&mut self) {
        if let Some(command) = self.transition(false) {
            spawn(command);
        }
    }

    fn transition(&mut self, idle: bool) -> Option<&str> {
        match (idle, self.fired) {
            (true, false) => {
                self.fired = true;
                Some(&self.command)
            }
            (false, true) => {
                self.fired = false;
                self.resume_command.as_deref()
            }
            _ => None,
        }
    }

    pub fn fired(&self) -> bool {
        self.fired
    }
}

pub fn load(path: &Path, session: Session) -> Result<Vec<ActiveRule>, String> {
    let source =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    parse(&source, session).map_err(|error| format!("{}: {error}", path.display()))
}

fn parse(source: &str, session: Session) -> Result<Vec<ActiveRule>, String> {
    let config: Config = toml::from_str(source).map_err(|error| error.to_string())?;
    if config.rule.is_empty() {
        return Err("at least one [[rule]] is required".to_owned());
    }
    config
        .rule
        .into_iter()
        .enumerate()
        .map(|(index, rule)| {
            let timeout_ms = rule
                .timeout
                .checked_mul(1000)
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    format!(
                        "rule {}: timeout must be between 1 and {} seconds",
                        index + 1,
                        u32::MAX / 1000
                    )
                })?;
            let command = match session {
                Session::X11 => rule.command_x11.or(rule.command),
                Session::Wayland => rule.command_wayland.or(rule.command),
            }
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("rule {}: no command for {}", index + 1, session.name()))?;
            if rule
                .resume_command
                .as_ref()
                .is_some_and(|s| s.trim().is_empty())
            {
                return Err(format!("rule {}: resume_command is empty", index + 1));
            }
            Ok(ActiveRule {
                timeout_ms,
                command,
                resume_command: rule.resume_command,
                fired: false,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_session_command_and_validates_timeout() {
        let source = "[[rule]]\ntimeout = 300\ncommand = 'amorphis'\ncommand_x11 = 'slock'\ncommand_wayland = 'kantele-locker'";
        assert_eq!(parse(source, Session::X11).unwrap()[0].command, "slock");
        assert_eq!(
            parse(source, Session::Wayland).unwrap()[0].command,
            "kantele-locker"
        );
        assert!(parse("[[rule]]\ntimeout = 0\ncommand = 'foo'", Session::X11).is_err());
        assert!(
            parse(
                "[[rule]]\ntimeout = 300\ncommand_x11 = 'slock'",
                Session::Wayland
            )
            .is_err()
        );
    }

    #[test]
    fn fires_once_per_idle_period_and_rearms_on_activity() {
        let mut rule = parse(
            "[[rule]]\ntimeout = 60\ncommand = 'lock'\nresume_command = 'active'",
            Session::Wayland,
        )
        .unwrap()
        .remove(0);
        assert_eq!(rule.transition(true), Some("lock"));
        assert_eq!(rule.transition(true), None);
        assert_eq!(rule.transition(false), Some("active"));
        assert_eq!(rule.transition(false), None);
        assert_eq!(rule.transition(true), Some("lock"));
        assert_eq!(rule.transition(false), Some("active"));
    }
}

fn spawn(command: &str) {
    match Command::new("sh").arg("-c").arg(command).spawn() {
        Ok(mut child) => {
            let command = command.to_owned();
            std::thread::spawn(move || match child.wait() {
                Ok(status) if !status.success() => {
                    eprintln!("amorphis-idle: command {command:?} exited with {status}")
                }
                Err(error) => {
                    eprintln!("amorphis-idle: could not reap command {command:?}: {error}")
                }
                _ => {}
            });
        }
        Err(error) => eprintln!("amorphis-idle: could not launch {command:?}: {error}"),
    }
}
