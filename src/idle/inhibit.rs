//! An open connection holds an inhibition for the current display session.

use super::IdleState;
use crate::session::Session;
use smithay_client_toolkit::reexports::calloop::{
    Interest, LoopHandle, Mode, PostAction, generic::Generic,
};
use std::collections::hash_map::DefaultHasher;
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::time::Duration;

const READY: u8 = 1;
const MAX_HOLDERS: usize = 64;

fn uid() -> libc::uid_t {
    // SAFETY: geteuid takes no arguments.
    unsafe { libc::geteuid() }
}

fn address(session: Session) -> io::Result<SocketAddr> {
    let mut hash = DefaultHasher::new();
    match session {
        Session::Wayland => {
            let display = std::env::var_os("WAYLAND_DISPLAY")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| OsString::from("wayland-0"));
            std::path::PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_default())
                .join(display)
                .hash(&mut hash);
        }
        Session::X11 => {
            let display = std::env::var("DISPLAY").unwrap_or_default();
            let server = display
                .rsplit_once('.')
                .filter(|(server, screen)| server.contains(':') && screen.parse::<u32>().is_ok())
                .map_or(display.as_str(), |(server, _)| server);
            server.hash(&mut hash);
        }
    }
    SocketAddr::from_abstract_name(format!(
        "amorphis-idle-{}-{}-{:016x}",
        uid(),
        session.name(),
        hash.finish()
    ))
}

fn check_peer(stream: &UnixStream) -> io::Result<()> {
    let mut credentials = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: getsockopt writes at most size bytes to the supplied buffer.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            credentials.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful SO_PEERCRED call with the full size initialized ucred.
    if size as usize != std::mem::size_of::<libc::ucred>()
        || unsafe { credentials.assume_init() }.uid != uid()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "idle inhibitor peer is not from the current user",
        ));
    }
    Ok(())
}

pub fn connect(session: Session) -> io::Result<UnixStream> {
    let mut stream = UnixStream::connect_addr(&address(session)?)?;
    check_peer(&stream)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut reply = [0];
    stream.read_exact(&mut reply)?;
    if reply != [READY] {
        return Err(io::Error::other("idle daemon refused the inhibition"));
    }
    Ok(stream)
}

pub fn listen(session: Session, handle: &LoopHandle<'_, IdleState>) -> Result<(), String> {
    let listener = UnixListener::bind_addr(&address(session).map_err(|e| e.to_string())?)
        .map_err(|error| format!("idle inhibitor socket: {error}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let register_stream = handle.downgrade();
    handle
        .insert_source(
            Generic::new(listener, Interest::READ, Mode::Level),
            move |_, listener, state| {
                let Some(register_stream) = register_stream.upgrade() else {
                    return Ok(PostAction::Remove);
                };
                for _ in 0..MAX_HOLDERS {
                    let (stream, _) = match listener.accept() {
                        Ok(connection) => connection,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) => return Err(error),
                    };
                    if state.manual_inhibitors >= MAX_HOLDERS
                        || check_peer(&stream).is_err()
                        || stream.set_nonblocking(true).is_err()
                    {
                        continue;
                    }
                    // Register before acknowledging so a failed registration cannot
                    // report an inhibition that the daemon cannot track.
                    let Ok(mut ack) = stream.try_clone() else {
                        continue;
                    };
                    let result = register_stream.insert_source(
                        Generic::new(stream, Interest::READ, Mode::Level),
                        |_, stream, state| match stream.as_ref().read(&mut [0]) {
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                                ) =>
                            {
                                Ok(PostAction::Continue)
                            }
                            _ => {
                                state.release();
                                Ok(PostAction::Remove)
                            }
                        },
                    );
                    match result {
                        Ok(token) => {
                            state.acquire();
                            if ack.write_all(&[READY]).is_err() {
                                register_stream.remove(token);
                                state.release();
                            }
                        }
                        Err(error) => eprintln!("amorphis-idle: idle inhibitor: {error}"),
                    }
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|error| format!("idle inhibitor socket: {error}"))?;
    Ok(())
}
