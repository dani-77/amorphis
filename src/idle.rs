//! Idle rules and command execution, independent of display protocols.

use crate::session::Session;
use serde::Deserialize;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

pub mod inhibit;
pub mod media;
pub mod wayland;

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
    #[serde(default)]
    inhibit: InhibitConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct InhibitConfig {
    #[serde(default)]
    players: Vec<String>,
}

pub struct IdleConfig {
    pub rules: Vec<ActiveRule>,
    pub players: Vec<String>,
}

pub struct IdleState {
    pub rules: Vec<ActiveRule>,
    manual_inhibitors: usize,
    playing: bool,
    pub restarted_at: Option<Instant>,
    pub restart_pending: bool,
}

impl IdleState {
    pub fn new(rules: Vec<ActiveRule>, playing: bool) -> Self {
        Self {
            rules,
            manual_inhibitors: 0,
            playing,
            restarted_at: None,
            restart_pending: false,
        }
    }

    pub fn inhibited(&self) -> bool {
        self.manual_inhibitors > 0 || self.playing
    }

    pub fn acquire(&mut self) {
        self.manual_inhibitors += 1;
    }

    pub fn release(&mut self) {
        self.manual_inhibitors -= 1;
        self.restart_if_uninhibited();
    }

    pub fn set_playing(&mut self, playing: bool) {
        let was_inhibited = self.inhibited();
        self.playing = playing;
        if was_inhibited {
            self.restart_if_uninhibited();
        }
    }

    fn restart_if_uninhibited(&mut self) {
        if !self.inhibited() {
            self.restarted_at = Some(Instant::now());
            self.restart_pending = true;
        }
    }
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

pub fn load(path: &Path, session: Session) -> Result<IdleConfig, String> {
    let source =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    parse(&source, session).map_err(|error| format!("{}: {error}", path.display()))
}

fn parse(source: &str, session: Session) -> Result<IdleConfig, String> {
    let config: Config = toml::from_str(source).map_err(|error| error.to_string())?;
    if config.rule.is_empty() {
        return Err("at least one [[rule]] is required".to_owned());
    }
    if config.inhibit.players.iter().any(|player| {
        player.is_empty()
            || !player
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_' | b'.'))
    }) {
        return Err("inhibit.players must contain MPRIS player names such as mpv or vlc".into());
    }
    let rules = config
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
        .collect::<Result<_, String>>()?;
    Ok(IdleConfig {
        rules,
        players: config.inhibit.players,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_session_command_and_validates_timeout() {
        let source = "[[rule]]\ntimeout = 300\ncommand = 'amorphis'\ncommand_x11 = 'slock'\ncommand_wayland = 'kantele-locker'";
        assert_eq!(
            parse(source, Session::X11).unwrap().rules[0].command,
            "slock"
        );
        assert_eq!(
            parse(source, Session::Wayland).unwrap().rules[0].command,
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
        .rules
        .remove(0);
        assert_eq!(rule.transition(true), Some("lock"));
        assert_eq!(rule.transition(true), None);
        assert_eq!(rule.transition(false), Some("active"));
        assert_eq!(rule.transition(false), None);
        assert_eq!(rule.transition(true), Some("lock"));
        assert_eq!(rule.transition(false), Some("active"));
    }

    #[test]
    fn only_the_last_inhibitor_restarts_idle() {
        let mut state = IdleState::new(Vec::new(), true);
        state.acquire();
        state.acquire();
        state.set_playing(false);
        state.release();
        assert!(state.inhibited());
        assert!(state.restarted_at.is_none());
        state.release();
        assert!(!state.inhibited());
        assert!(state.restarted_at.is_some());
        assert!(state.restart_pending);
    }
}
