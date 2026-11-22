use amorphis::idle::{self, ActiveRule};
use amorphis::session::{self, Session};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("Usage: amorphis-idle\nReads idle.toml from the amorphis config directory.");
        return ExitCode::SUCCESS;
    }
    if args == ["--version"] || args == ["-V"] {
        println!("amorphis-idle {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if !args.is_empty() {
        eprintln!("amorphis-idle: invalid arguments");
        return ExitCode::from(2);
    }
    let result = (|| {
        let session = Session::detect().map_err(str::to_owned)?;
        let path = session::config_dir()
            .map_err(str::to_owned)?
            .join("idle.toml");
        let rules = idle::load(&path, session)?;
        match session {
            Session::X11 => x11(rules),
            Session::Wayland => wayland(rules),
        }
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("amorphis-idle: {error}");
            ExitCode::from(1)
        }
    }
}

fn x11(mut rules: Vec<ActiveRule>) -> Result<(), String> {
    use x11rb::connection::Connection;
    use x11rb::protocol::screensaver::ConnectionExt;
    let (conn, screen) =
        x11rb::connect(None).map_err(|error| format!("X11 connection: {error}"))?;
    let root = conn.setup().roots[screen].root;
    conn.screensaver_query_version(1, 1)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|error| format!("MIT-SCREEN-SAVER unavailable: {error}"))?;
    loop {
        let idle_ms = conn
            .screensaver_query_info(root)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|error| format!("MIT-SCREEN-SAVER query failed: {error}"))?
            .ms_since_user_input;
        for rule in &mut rules {
            if idle_ms >= rule.timeout_ms {
                rule.idled();
            } else {
                rule.resumed();
            }
        }
        let next = rules
            .iter()
            .filter(|r| !r.fired())
            .map(|r| r.timeout_ms.saturating_sub(idle_ms))
            .min()
            .unwrap_or(1000);
        std::thread::sleep(std::time::Duration::from_millis(u64::from(
            next.clamp(100, 1000),
        )));
    }
}

fn wayland(rules: Vec<ActiveRule>) -> Result<(), String> {
    use wayland_client::{
        Connection, Dispatch, QueueHandle,
        globals::{GlobalListContents, registry_queue_init},
        protocol::{wl_registry, wl_seat},
    };
    use wayland_protocols::ext::idle_notify::v1::client::{
        ext_idle_notification_v1, ext_idle_notifier_v1,
    };

    struct State {
        rules: Vec<ActiveRule>,
    }
    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    impl Dispatch<wl_seat::WlSeat, ()> for State {
        fn event(
            _: &mut Self,
            _: &wl_seat::WlSeat,
            _: wl_seat::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    impl Dispatch<ext_idle_notifier_v1::ExtIdleNotifierV1, ()> for State {
        fn event(
            _: &mut Self,
            _: &ext_idle_notifier_v1::ExtIdleNotifierV1,
            _: ext_idle_notifier_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, usize> for State {
        fn event(
            state: &mut Self,
            _: &ext_idle_notification_v1::ExtIdleNotificationV1,
            event: ext_idle_notification_v1::Event,
            index: &usize,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                ext_idle_notification_v1::Event::Idled => state.rules[*index].idled(),
                ext_idle_notification_v1::Event::Resumed => state.rules[*index].resumed(),
                _ => {}
            }
        }
    }
    let conn = Connection::connect_to_env().map_err(|e| format!("Wayland connection: {e}"))?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&conn).map_err(|e| format!("Wayland registry: {e}"))?;
    let qh = queue.handle();
    let notifier: ext_idle_notifier_v1::ExtIdleNotifierV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|e| format!("ext-idle-notify-v1 unavailable: {e}"))?;
    let seat: wl_seat::WlSeat = globals
        .bind(&qh, 1..=1, ())
        .map_err(|e| format!("Wayland seat unavailable: {e}"))?;
    let mut state = State { rules };
    let _notifications: Vec<_> = state
        .rules
        .iter()
        .enumerate()
        .map(|(index, rule)| notifier.get_idle_notification(rule.timeout_ms, &seat, &qh, index))
        .collect();
    loop {
        queue
            .blocking_dispatch(&mut state)
            .map_err(|e| format!("Wayland event dispatch: {e}"))?;
    }
}
