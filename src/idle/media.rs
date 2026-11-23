//! Optional MPRIS playback detection, sampled off the display event loop.

use super::IdleState;
use smithay_client_toolkit::reexports::calloop::{LoopHandle, channel};
use std::io;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(1);
const QUERY_TIMEOUT: Duration = Duration::from_millis(750);

pub fn watch(players: &[String], handle: &LoopHandle<'_, IdleState>) -> Result<bool, String> {
    if players.is_empty() {
        return Ok(false);
    }
    let names = players.join(",");
    let initial = match query(&names) {
        Ok(playing) => playing,
        Err(error) => {
            eprintln!("amorphis-idle: playerctl: {error}");
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(false);
            }
            false
        }
    };
    let (tx, rx) = channel::channel();
    handle
        .insert_source(rx, |event, _, state| {
            state.set_playing(match event {
                channel::Event::Msg(playing) => playing,
                channel::Event::Closed => false,
            });
        })
        .map_err(|error| format!("media monitoring: {error}"))?;
    std::thread::Builder::new()
        .name("amorphis-media".into())
        .spawn(move || {
            let mut previous = initial;
            let mut logged_error = false;
            loop {
                std::thread::sleep(INTERVAL);
                let playing = match query(&names) {
                    Ok(playing) => {
                        logged_error = false;
                        playing
                    }
                    Err(error) => {
                        if !logged_error {
                            eprintln!("amorphis-idle: playerctl: {error}");
                            logged_error = true;
                        }
                        false
                    }
                };
                if playing != previous {
                    if tx.send(playing).is_err() {
                        break;
                    }
                    previous = playing;
                }
            }
        })
        .map_err(|error| format!("media monitoring: {error}"))?;
    Ok(initial)
}

fn query(names: &str) -> io::Result<bool> {
    let mut child = Command::new("playerctl")
        .args([
            "--all-players",
            "--no-messages",
            "--player",
            names,
            "status",
        ])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + QUERY_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(result.err().unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::TimedOut, "playback query timed out")
                }));
            }
        }
    }
    let output = child.wait_with_output()?;
    // playerctl returns 1 when there are no matching players.
    Ok(output.status.success()
        && output
            .stdout
            .split(|byte| *byte == b'\n')
            .any(|line| line == b"Playing"))
}
