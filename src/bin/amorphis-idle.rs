use amorphis::idle::{self, IdleState, inhibit, media, wayland};
use amorphis::session::{self, Session};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use std::ffi::OsString;
use std::io::{self, Read};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitCode};
use std::time::Duration;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!(
            "Usage: amorphis-idle\n       amorphis-idle inhibit --for SECONDS\n       amorphis-idle inhibit -- COMMAND [ARG...]\n\nThe daemon reads idle.toml from the amorphis config directory."
        );
        return ExitCode::SUCCESS;
    }
    if args == ["--version"] || args == ["-V"] {
        println!("amorphis-idle {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let result = match args.as_slice() {
        [] => run_daemon().map(|()| ExitCode::SUCCESS),
        [inhib, flag, seconds] if inhib == "inhibit" && flag == "--for" => {
            match seconds.to_str().and_then(|value| value.parse::<u32>().ok()) {
                Some(seconds) if seconds > 0 => {
                    inhibit_for(Duration::from_secs(u64::from(seconds)))
                }
                _ => {
                    eprintln!("amorphis-idle: --for requires a positive number of seconds");
                    return ExitCode::from(2);
                }
            }
        }
        [inhib, separator, command, arguments @ ..] if inhib == "inhibit" && separator == "--" => {
            inhibit_command(command, arguments)
        }
        _ => {
            eprintln!("amorphis-idle: invalid arguments; see --help");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("amorphis-idle: {error}");
            ExitCode::from(1)
        }
    }
}

fn connect_inhibitor() -> Result<std::os::unix::net::UnixStream, String> {
    let session = Session::detect().map_err(str::to_owned)?;
    inhibit::connect(session)
        .map_err(|error| format!("cannot inhibit the idle daemon for this session: {error}"))
}

fn inhibit_for(duration: Duration) -> Result<ExitCode, String> {
    let mut lease = connect_inhibitor()?;
    lease
        .set_read_timeout(Some(duration))
        .map_err(|error| error.to_string())?;
    match lease.read(&mut [0]) {
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Ok(ExitCode::SUCCESS)
        }
        Ok(_) => Err("idle daemon closed the inhibition early".into()),
        Err(error) => Err(format!("idle inhibitor: {error}")),
    }
}

fn inhibit_command(command: &OsString, arguments: &[OsString]) -> Result<ExitCode, String> {
    let _lease = connect_inhibitor()?;
    let status = Command::new(command)
        .args(arguments)
        .status()
        .map_err(|error| format!("could not run {command:?}: {error}"))?;
    let code = status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
    Ok(ExitCode::from(code as u8))
}

fn run_daemon() -> Result<(), String> {
    let session = Session::detect().map_err(str::to_owned)?;
    let path = session::config_dir()
        .map_err(str::to_owned)?
        .join("idle.toml");
    let config = idle::load(&path, session)?;
    let mut event_loop = EventLoop::<IdleState>::try_new().map_err(|error| error.to_string())?;
    let handle = event_loop.handle();
    inhibit::listen(session, &handle)?;
    let playing = media::watch(&config.players, &handle)?;
    let state = IdleState::new(config.rules, playing);
    match session {
        Session::X11 => x11(&mut event_loop, state),
        Session::Wayland => wayland::run(&mut event_loop, state),
    }
}

fn x11(event_loop: &mut EventLoop<'_, IdleState>, mut state: IdleState) -> Result<(), String> {
    use x11rb::connection::Connection;
    use x11rb::protocol::screensaver::ConnectionExt;
    let (conn, screen) =
        x11rb::connect(None).map_err(|error| format!("X11 connection: {error}"))?;
    let root = conn.setup().roots[screen].root;
    conn.screensaver_query_version(1, 1)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|error| format!("MIT-SCREEN-SAVER unavailable: {error}"))?;
    let mut delay = Duration::ZERO;
    loop {
        event_loop
            .dispatch(delay, &mut state)
            .map_err(|error| error.to_string())?;
        let idle_ms = conn
            .screensaver_query_info(root)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|error| format!("MIT-SCREEN-SAVER query failed: {error}"))?
            .ms_since_user_input;
        let inhibited = state.inhibited();
        let eligible_ms = state.restarted_at.map_or(idle_ms, |restart| {
            idle_ms.min(restart.elapsed().as_millis().try_into().unwrap_or(u32::MAX))
        });
        for rule in &mut state.rules {
            if idle_ms < rule.timeout_ms {
                rule.resumed();
            } else if !inhibited && eligible_ms >= rule.timeout_ms {
                rule.idled();
            }
        }
        let next = state
            .rules
            .iter()
            .filter(|r| !inhibited && !r.fired())
            .map(|r| r.timeout_ms.saturating_sub(eligible_ms))
            .min()
            .unwrap_or(1000);
        delay = Duration::from_millis(u64::from(next.clamp(100, 1000)));
    }
}
