//! Wayland backend using `ext-session-lock-v1` exclusively.

use crate::auth::{AuthOutcome, AuthWorker, PamProvider, SubmitError};
use crate::backend::wayland_capture::{CapturedOutput, capture_outputs};
use crate::background::{
    BackgroundSource, LOCK_COLOR, StaticBackground, load_background, start_loader,
};
use crate::image::{FitMode, ImageEffect, ImageLimits, RgbaImage};
use crate::indicator::{AuthIndicator, IndicatorStyle, draw_indicator};
use crate::secret::SecretBuffer;
use crate::state::{LockEvent, LockMachine, LockState};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputData, OutputHandler, OutputState},
    reexports::{calloop::EventLoop, calloop_wayland_source::WaylandSource},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
    },
    session_lock::{
        SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
        SessionLockSurfaceConfigure,
    },
    shm::{Shm, ShmHandler, raw::RawPool},
};
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;
use wayland_client::{
    Connection, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_buffer, wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface},
};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);
#[derive(Debug)]
pub enum WaylandError {
    Connect(String),
    Registry(String),
    MissingGlobal(&'static str),
    EventLoop(String),
    EventSource(String),
    LockProtocolUnavailable,
    LockRefused,
    AuthenticationWorker(String),
    AuthenticationStopped,
    ProtocolState,
    ConnectionLost(String),
}

impl fmt::Display for WaylandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(message) => write!(f, "could not connect to Wayland: {message}"),
            Self::Registry(message) => write!(f, "could not read Wayland globals: {message}"),
            Self::MissingGlobal(name) => write!(f, "required Wayland global is absent: {name}"),
            Self::EventLoop(message) => write!(f, "could not create event loop: {message}"),
            Self::EventSource(message) => {
                write!(f, "could not attach Wayland event source: {message}")
            }
            Self::LockProtocolUnavailable => {
                f.write_str("compositor does not support ext-session-lock-v1")
            }
            Self::LockRefused => f.write_str("compositor refused the session lock request"),
            Self::AuthenticationWorker(message) => {
                write!(f, "could not create authentication worker: {message}")
            }
            Self::AuthenticationStopped => {
                f.write_str("authentication worker stopped while the session was locked")
            }
            Self::ProtocolState => f.write_str("invalid internal lock-state transition"),
            Self::ConnectionLost(message) => write!(f, "Wayland connection failed: {message}"),
        }
    }
}

impl Error for WaylandError {}

/// Lock the current Wayland session and run until successful PAM
/// authentication. This function never falls back to layer-shell or XWayland.
pub fn run(
    pam_service: &str,
    username: &str,
    static_background: Option<StaticBackground>,
) -> Result<(), WaylandError> {
    let limits = ImageLimits::default();
    let PreparedBackground {
        shared_rx,
        pending_capture,
        fit,
    } = prepare_background(static_background, limits);
    let conn =
        Connection::connect_to_env().map_err(|error| WaylandError::Connect(error.to_string()))?;
    let (globals, event_queue) =
        registry_queue_init(&conn).map_err(|error| WaylandError::Registry(error.to_string()))?;
    let qh: QueueHandle<WaylandApp> = event_queue.handle();

    let compositor_state = CompositorState::bind(&globals, &qh)
        .map_err(|_| WaylandError::MissingGlobal("wl_compositor"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|_| WaylandError::MissingGlobal("wl_shm"))?;
    let session_lock_state = SessionLockState::new(&globals, &qh);
    let auth = AuthWorker::spawn(PamProvider::new(pam_service, username))
        .map_err(|error| WaylandError::AuthenticationWorker(error.to_string()))?;
    let secret = SecretBuffer::standard()
        .map_err(|error| WaylandError::AuthenticationWorker(error.to_string()))?;
    if !secret.is_memory_locked() {
        eprintln!("amorphis: warning: password buffer could not be locked against swap");
    }
    let mut event_loop: EventLoop<WaylandApp> =
        EventLoop::try_new().map_err(|error| WaylandError::EventLoop(error.to_string()))?;

    let mut app = WaylandApp {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor_state,
        shm,
        session_lock_state,
        session_lock: None,
        surfaces: Vec::new(),
        keyboards: Vec::new(),
        modifiers: Modifiers::default(),
        machine: LockMachine::new(),
        auth,
        active_attempt: None,
        secret,
        exit: None,
        limits,
        background_rx: shared_rx,
        capture_rx: None,
        background_fit: fit,
        background_image: None,
        captured_images: HashMap::new(),
        indicator: AuthIndicator::default(),
    };

    app.machine
        .apply(LockEvent::RequestLock)
        .map_err(|_| WaylandError::ProtocolState)?;
    let lock = match app.session_lock_state.lock(&qh) {
        Ok(lock) => lock,
        Err(_) => {
            app.machine
                .apply(LockEvent::LockRefused)
                .map_err(|_| WaylandError::ProtocolState)?;
            return Err(WaylandError::LockProtocolUnavailable);
        }
    };
    app.session_lock = Some(lock);
    if let Some((outputs, effect)) = pending_capture {
        match start_capture_processor(outputs, effect, limits) {
            Ok(receiver) => app.capture_rx = Some(receiver),
            Err(error) => {
                eprintln!("amorphis: {error}; using the embedded fallback background");
                let fallback = StaticBackground {
                    source: BackgroundSource::EmbeddedFallback,
                    fit,
                    effect,
                };
                (app.background_rx, _) = start_loader(Some(fallback), limits);
            }
        }
    }

    let initial_outputs: Vec<_> = app.output_state.outputs().collect();
    for output in initial_outputs {
        app.add_output(&qh, output);
    }

    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .map_err(|error| WaylandError::EventSource(error.to_string()))?;

    loop {
        event_loop
            .dispatch(EVENT_POLL_INTERVAL, &mut app)
            .map_err(|error| WaylandError::ConnectionLost(error.to_string()))?;
        if let Some(exit) = app.exit.take() {
            return exit;
        }
        app.poll_background(&qh);
        app.poll_capture(&qh);
        app.poll_authentication(&conn)?;

        if let Some(exit) = app.exit.take() {
            return exit;
        }
        app.poll_indicator(&qh);
    }
}

struct PreparedBackground {
    shared_rx: Option<Receiver<Result<RgbaImage, String>>>,
    pending_capture: Option<(Vec<CapturedOutput>, ImageEffect)>,
    fit: FitMode,
}

fn prepare_background(
    static_background: Option<StaticBackground>,
    limits: ImageLimits,
) -> PreparedBackground {
    let Some(mut background) = static_background else {
        return PreparedBackground {
            shared_rx: None,
            pending_capture: None,
            fit: FitMode::Cover,
        };
    };
    if background.source != BackgroundSource::DesktopCapture {
        let (receiver, fit) = start_loader(Some(background), limits);
        return PreparedBackground {
            shared_rx: receiver,
            pending_capture: None,
            fit,
        };
    }

    let fit = background.fit;
    let effect = background.effect;
    match capture_outputs(limits) {
        Ok(outputs) => PreparedBackground {
            shared_rx: None,
            pending_capture: Some((outputs, effect)),
            fit,
        },
        Err(error) => {
            eprintln!("amorphis: {error}; using the embedded fallback background");
            background.source = BackgroundSource::EmbeddedFallback;
            let (receiver, _) = start_loader(Some(background), limits);
            PreparedBackground {
                shared_rx: receiver,
                pending_capture: None,
                fit,
            }
        }
    }
}

fn start_capture_processor(
    mut outputs: Vec<CapturedOutput>,
    effect: ImageEffect,
    limits: ImageLimits,
) -> Result<Receiver<Result<CaptureBackground, String>>, String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("amorphis-capture".into())
        .spawn(move || {
            let capture_result = outputs.iter_mut().try_for_each(|output| {
                effect
                    .apply(&mut output.image, limits)
                    .map_err(|error| format!("could not apply capture effect: {error}"))
            });
            let result = match capture_result {
                Ok(()) => Ok(CaptureBackground::PerOutput(outputs)),
                Err(capture_error) => {
                    eprintln!("amorphis: {capture_error}; using the embedded fallback background");
                    load_background(&BackgroundSource::EmbeddedFallback, limits).and_then(
                        |mut image| {
                            effect.apply(&mut image, limits).map_err(|error| {
                                format!("could not apply fallback image effect: {error}")
                            })?;
                            Ok(CaptureBackground::SharedFallback(image))
                        },
                    )
                }
            };
            let _ = sender.send(result);
        })
        .map_err(|error| format!("could not start Wayland capture processor: {error}"))?;
    Ok(receiver)
}

struct LockedOutput {
    output: wl_output::WlOutput,
    output_id: Option<u32>,
    // Retaining the protocol object keeps this output covered.
    _surface: SessionLockSurface,
    configured_size: Option<(u32, u32)>,
}

struct WaylandApp {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor_state: CompositorState,
    shm: Shm,
    session_lock_state: SessionLockState,
    session_lock: Option<SessionLock>,
    surfaces: Vec<LockedOutput>,
    keyboards: Vec<(wl_seat::WlSeat, wl_keyboard::WlKeyboard)>,
    modifiers: Modifiers,
    machine: LockMachine,
    auth: AuthWorker,
    active_attempt: Option<u64>,
    secret: SecretBuffer,
    exit: Option<Result<(), WaylandError>>,
    limits: ImageLimits,
    background_rx: Option<Receiver<Result<RgbaImage, String>>>,
    capture_rx: Option<Receiver<Result<CaptureBackground, String>>>,
    background_fit: FitMode,
    background_image: Option<RgbaImage>,
    captured_images: HashMap<u32, RgbaImage>,
    indicator: AuthIndicator,
}

enum CaptureBackground {
    PerOutput(Vec<CapturedOutput>),
    SharedFallback(RgbaImage),
}

impl WaylandApp {
    fn add_output(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if self.surfaces.iter().any(|entry| entry.output == output) {
            return;
        }
        let Some(lock) = self.session_lock.as_ref() else {
            return;
        };
        let surface = self.compositor_state.create_surface(qh);
        let surface = lock.create_lock_surface(surface, &output, qh);
        let output_id = output
            .data::<OutputData>()
            .map(|data| data.with_output_info(|info| info.id));
        self.surfaces.push(LockedOutput {
            output,
            output_id,
            _surface: surface,
            configured_size: None,
        });
    }

    fn render_current(
        &mut self,
        qh: &QueueHandle<Self>,
        surface: &SessionLockSurface,
        width: u32,
        height: u32,
        output_id: Option<u32>,
    ) -> Result<(), &'static str> {
        let style = self.indicator.style();
        self.render_current_with_indicator(qh, surface, width, height, output_id, style)
    }

    fn render_current_with_indicator(
        &mut self,
        qh: &QueueHandle<Self>,
        surface: &SessionLockSurface,
        width: u32,
        height: u32,
        output_id: Option<u32>,
        style: IndicatorStyle,
    ) -> Result<(), &'static str> {
        let source = background_for_output(
            &self.captured_images,
            self.background_image.as_ref(),
            output_id,
        );
        let mut frame = match source {
            Some(source) => source
                .fitted(width, height, self.background_fit, LOCK_COLOR, self.limits)
                .or_else(|_| RgbaImage::solid(width, height, LOCK_COLOR, self.limits)),
            None => RgbaImage::solid(width, height, LOCK_COLOR, self.limits),
        }
        .map_err(|_| "could not allocate lock frame")?;
        draw_indicator(&mut frame, style);
        self.render_rgba(qh, surface, &frame)
    }

    fn render_solid(
        &mut self,
        qh: &QueueHandle<Self>,
        surface: &SessionLockSurface,
        width: u32,
        height: u32,
    ) -> Result<(), &'static str> {
        let len = checked_argb_len(width, height, self.limits).ok_or("invalid lock dimensions")?;
        let width_i32 = i32::try_from(width).map_err(|_| "lock width exceeds protocol range")?;
        let height_i32 = i32::try_from(height).map_err(|_| "lock height exceeds protocol range")?;
        let stride = width_i32.checked_mul(4).ok_or("lock stride overflow")?;
        let mut pool =
            RawPool::new(len, &self.shm).map_err(|_| "could not allocate lock buffer")?;
        let color = argb8888_bytes(LOCK_COLOR);
        for pixel in pool.mmap().as_chunks_mut::<4>().0 {
            *pixel = color;
        }
        let buffer = pool.create_buffer(
            0,
            width_i32,
            height_i32,
            stride,
            wl_shm::Format::Argb8888,
            (),
            qh,
        );
        surface.wl_surface().attach(Some(&buffer), 0, 0);
        surface
            .wl_surface()
            .damage_buffer(0, 0, width_i32, height_i32);
        surface.wl_surface().commit();
        buffer.destroy();
        Ok(())
    }

    fn render_rgba(
        &mut self,
        qh: &QueueHandle<Self>,
        surface: &SessionLockSurface,
        image: &RgbaImage,
    ) -> Result<(), &'static str> {
        let width = image.width();
        let height = image.height();
        let len = checked_argb_len(width, height, self.limits).ok_or("invalid lock dimensions")?;
        let width_i32 = i32::try_from(width).map_err(|_| "lock width exceeds protocol range")?;
        let height_i32 = i32::try_from(height).map_err(|_| "lock height exceeds protocol range")?;
        let stride = width_i32.checked_mul(4).ok_or("lock stride overflow")?;
        let mut pool =
            RawPool::new(len, &self.shm).map_err(|_| "could not allocate lock buffer")?;
        for (target, source) in pool
            .mmap()
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(image.pixels().as_chunks::<4>().0)
        {
            *target = rgba_over_opaque_argb(*source, LOCK_COLOR);
        }
        let buffer = pool.create_buffer(
            0,
            width_i32,
            height_i32,
            stride,
            wl_shm::Format::Argb8888,
            (),
            qh,
        );
        surface.wl_surface().attach(Some(&buffer), 0, 0);
        surface
            .wl_surface()
            .damage_buffer(0, 0, width_i32, height_i32);
        surface.wl_surface().commit();
        buffer.destroy();
        Ok(())
    }

    fn poll_background(&mut self, qh: &QueueHandle<Self>) {
        let Some(receiver) = self.background_rx.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.background_rx = None;
                eprintln!("amorphis: PNG loader stopped; retaining solid background");
                return;
            }
        };
        self.background_rx = None;

        match result {
            Ok(image) => {
                self.background_image = Some(image);
                let surfaces: Vec<_> = self
                    .surfaces
                    .iter()
                    .filter_map(|entry| {
                        entry
                            .configured_size
                            .map(|size| (entry._surface.clone(), size, entry.output_id))
                    })
                    .collect();
                for (surface, (width, height), output_id) in surfaces {
                    if self
                        .render_current(qh, &surface, width, height, output_id)
                        .is_err()
                    {
                        eprintln!(
                            "amorphis: could not draw static PNG; retaining the previous opaque frame"
                        );
                    }
                }
            }
            Err(error) => {
                eprintln!("amorphis: {error}; retaining solid background");
            }
        }
    }

    fn poll_capture(&mut self, qh: &QueueHandle<Self>) {
        let Some(receiver) = self.capture_rx.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.capture_rx = None;
                eprintln!(
                    "amorphis: Wayland capture processor stopped; retaining solid background"
                );
                return;
            }
        };
        self.capture_rx = None;

        match result {
            Ok(CaptureBackground::PerOutput(outputs)) => {
                self.captured_images = outputs
                    .into_iter()
                    .map(|output| (output.output_id, output.image))
                    .collect();
            }
            Ok(CaptureBackground::SharedFallback(image)) => {
                self.background_image = Some(image);
            }
            Err(error) => {
                eprintln!("amorphis: {error}; retaining solid background");
                return;
            }
        }

        let surfaces: Vec<_> = self
            .surfaces
            .iter()
            .filter_map(|entry| {
                entry
                    .configured_size
                    .map(|size| (entry._surface.clone(), size, entry.output_id))
            })
            .collect();
        for (surface, (width, height), output_id) in surfaces {
            if self
                .render_current(qh, &surface, width, height, output_id)
                .is_err()
            {
                eprintln!(
                    "amorphis: could not draw captured desktop; retaining the previous opaque frame"
                );
            }
        }
    }

    fn poll_indicator(&mut self, qh: &QueueHandle<Self>) {
        let Some(style) = self.indicator.take_redraw() else {
            return;
        };
        let surfaces: Vec<_> = self
            .surfaces
            .iter()
            .filter_map(|entry| {
                entry
                    .configured_size
                    .map(|size| (entry._surface.clone(), size, entry.output_id))
            })
            .collect();
        for (surface, (width, height), output_id) in surfaces {
            if self
                .render_current_with_indicator(qh, &surface, width, height, output_id, style)
                .is_err()
            {
                eprintln!("amorphis: could not redraw authentication indicator");
            }
        }
    }

    fn handle_key(&mut self, event: KeyEvent) {
        if self.machine.state() != LockState::Locked {
            return;
        }

        match event.keysym {
            Keysym::Escape => {
                self.secret.clear();
                self.indicator.input();
            }
            Keysym::BackSpace => {
                self.secret.pop_char();
                self.indicator.input();
            }
            Keysym::Return | Keysym::KP_Enter => self.submit_authentication(),
            Keysym::u if self.modifiers.ctrl => {
                self.secret.clear();
                self.indicator.input();
            }
            _ if !self.modifiers.ctrl && !self.modifiers.alt && !self.modifiers.logo => {
                if let Some(text) = event.utf8
                    && text.chars().all(|character| !character.is_control())
                    && !text.is_empty()
                    && self.secret.push_str(&text).is_ok()
                {
                    self.indicator.input();
                }
            }
            _ => {}
        }
    }

    fn submit_authentication(&mut self) {
        let replacement = match SecretBuffer::standard() {
            Ok(secret) => secret,
            Err(_) => return,
        };
        if !replacement.is_memory_locked() {
            eprintln!(
                "amorphis: warning: replacement password buffer could not be locked against swap"
            );
        }
        let submitted = std::mem::replace(&mut self.secret, replacement);
        match self.auth.submit(submitted) {
            Ok(attempt) => {
                if self.machine.apply(LockEvent::BeginAuthentication).is_err() {
                    self.exit = Some(Err(WaylandError::ProtocolState));
                    return;
                }
                self.active_attempt = Some(attempt);
                self.indicator.verifying();
            }
            Err(SubmitError::Busy(secret)) => {
                self.secret = secret;
            }
            Err(SubmitError::Stopped(secret)) => {
                self.secret = secret;
                self.exit = Some(Err(WaylandError::AuthenticationStopped));
            }
        }
    }

    fn poll_authentication(&mut self, conn: &Connection) -> Result<(), WaylandError> {
        let response = self
            .auth
            .try_response()
            .map_err(|_| WaylandError::AuthenticationStopped)?;
        let Some(response) = response else {
            return Ok(());
        };
        if self.active_attempt.take() != Some(response.attempt) {
            return Err(WaylandError::ProtocolState);
        }

        match response.outcome {
            AuthOutcome::Success => {
                self.machine
                    .apply(LockEvent::AuthenticationSucceeded)
                    .map_err(|_| WaylandError::ProtocolState)?;
                self.machine
                    .apply(LockEvent::BeginUnlock)
                    .map_err(|_| WaylandError::ProtocolState)?;
                self.session_lock
                    .as_ref()
                    .ok_or(WaylandError::ProtocolState)?
                    .unlock();
                conn.roundtrip()
                    .map_err(|error| WaylandError::ConnectionLost(error.to_string()))?;
                self.machine
                    .apply(LockEvent::UnlockCompleted)
                    .map_err(|_| WaylandError::ProtocolState)?;
                self.exit = Some(Ok(()));
            }
            AuthOutcome::Denied { .. } | AuthOutcome::Error { .. } => {
                self.machine
                    .apply(LockEvent::AuthenticationFailed)
                    .map_err(|_| WaylandError::ProtocolState)?;
                self.indicator.failed();
            }
        }
        Ok(())
    }
}

fn background_for_output<'a>(
    captured: &'a HashMap<u32, RgbaImage>,
    shared: Option<&'a RgbaImage>,
    output_id: Option<u32>,
) -> Option<&'a RgbaImage> {
    output_id.and_then(|id| captured.get(&id)).or(shared)
}

impl SessionLockHandler for WaylandApp {
    fn locked(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _lock: SessionLock) {
        if self.machine.apply(LockEvent::LockConfirmed).is_err() {
            self.exit = Some(Err(WaylandError::ProtocolState));
        }
    }

    fn finished(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _lock: SessionLock) {
        if self.machine.state() == LockState::LockRequested {
            let _ = self.machine.apply(LockEvent::LockRefused);
            self.exit = Some(Err(WaylandError::LockRefused));
        } else {
            // Never translate a compositor-side termination into an unlock.
            self.exit = Some(Err(WaylandError::ProtocolState));
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _serial: u32,
    ) {
        let (width, height) = configure.new_size;
        let output_id = self
            .surfaces
            .iter()
            .find(|entry| entry._surface.wl_surface() == surface.wl_surface())
            .and_then(|entry| entry.output_id);
        let first_frame = self
            .surfaces
            .iter()
            .find(|entry| entry._surface.wl_surface() == surface.wl_surface())
            .is_none_or(|entry| entry.configured_size.is_none());
        let rendered = if first_frame {
            self.render_solid(qh, &surface, width, height)
        } else {
            self.render_current(qh, &surface, width, height, output_id)
        };
        if rendered.is_ok()
            && let Some(entry) = self
                .surfaces
                .iter_mut()
                .find(|entry| entry._surface.wl_surface() == surface.wl_surface())
        {
            entry.configured_size = Some((width, height));
        }
        if rendered.is_err() && self.machine.state() == LockState::LockRequested {
            self.exit = Some(Err(WaylandError::ProtocolState));
        }
    }
}

impl CompositorHandler for WaylandApp {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for WaylandApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.add_output(qh, output);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.surfaces.retain(|entry| entry.output != output);
    }
}

impl SeatHandler for WaylandApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && !self.keyboards.iter().any(|(known, _)| *known == seat)
            && let Ok(keyboard) = self.seat_state.get_keyboard(qh, &seat, None)
        {
            self.keyboards.push((seat, keyboard));
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            self.remove_keyboard(&seat);
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.remove_keyboard(&seat);
    }
}

impl WaylandApp {
    fn remove_keyboard(&mut self, seat: &wl_seat::WlSeat) {
        if let Some(index) = self.keyboards.iter().position(|(known, _)| known == seat) {
            let (_, keyboard) = self.keyboards.swap_remove(index);
            keyboard.release();
        }
    }
}

impl KeyboardHandler for WaylandApp {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        self.handle_key(event);
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        self.handle_key(event);
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _raw_modifiers: RawModifiers,
        _layout: u32,
    ) {
        self.modifiers = modifiers;
    }
}

impl ProvidesRegistryState for WaylandApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

impl ShmHandler for WaylandApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

fn checked_argb_len(width: u32, height: u32, limits: ImageLimits) -> Option<usize> {
    if width == 0 || height == 0 || width > limits.max_width || height > limits.max_height {
        return None;
    }
    let pixels = u64::from(width).checked_mul(u64::from(height))?;
    if pixels > limits.max_pixels {
        return None;
    }
    let bytes = usize::try_from(pixels.checked_mul(4)?).ok()?;
    (bytes <= limits.max_bytes).then_some(bytes)
}

fn argb8888_bytes(rgba: [u8; 4]) -> [u8; 4] {
    let [red, green, blue, alpha] = rgba;
    (u32::from(alpha) << 24 | u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue))
        .to_ne_bytes()
}

fn rgba_over_opaque_argb(source: [u8; 4], background: [u8; 4]) -> [u8; 4] {
    let alpha = u32::from(source[3]);
    let inverse = 255 - alpha;
    let mut opaque = [0_u8; 4];
    for channel in 0..3 {
        opaque[channel] =
            ((u32::from(source[channel]) * alpha + u32::from(background[channel]) * inverse + 127)
                / 255) as u8;
    }
    opaque[3] = 255;
    argb8888_bytes(opaque)
}

smithay_client_toolkit::delegate_compositor!(WaylandApp);
smithay_client_toolkit::delegate_output!(WaylandApp);
smithay_client_toolkit::delegate_seat!(WaylandApp);
smithay_client_toolkit::delegate_keyboard!(WaylandApp);
smithay_client_toolkit::delegate_session_lock!(WaylandApp);
smithay_client_toolkit::delegate_shm!(WaylandApp);
smithay_client_toolkit::delegate_registry!(WaylandApp);
wayland_client::delegate_noop!(WaylandApp: ignore wl_buffer::WlBuffer);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::{BackgroundSource, load_background};

    #[test]
    fn argb_encoding_matches_native_wayland_layout() {
        let bytes = argb8888_bytes([0x11, 0x22, 0x33, 0xff]);
        assert_eq!(u32::from_ne_bytes(bytes), 0xff11_2233);
    }

    #[test]
    fn buffer_length_checks_dimensions_and_limits() {
        let limits = ImageLimits {
            max_width: 10,
            max_height: 10,
            max_pixels: 100,
            max_bytes: 400,
            ..ImageLimits::default()
        };
        assert_eq!(checked_argb_len(10, 10, limits), Some(400));
        assert_eq!(checked_argb_len(0, 10, limits), None);
        assert_eq!(checked_argb_len(11, 1, limits), None);
    }

    #[test]
    fn transparent_pixels_are_composited_to_an_opaque_wayland_pixel() {
        let bytes = rgba_over_opaque_argb([200, 100, 50, 0], [10, 20, 30, 255]);
        assert_eq!(u32::from_ne_bytes(bytes), 0xff0a_141e);

        let bytes = rgba_over_opaque_argb([200, 100, 50, 255], [10, 20, 30, 255]);
        assert_eq!(u32::from_ne_bytes(bytes), 0xffc8_6432);
    }

    #[test]
    fn embedded_fallback_decodes_under_production_limits() {
        let image = load_background(&BackgroundSource::EmbeddedFallback, ImageLimits::default())
            .expect("embedded fallback must remain a valid bounded PNG");
        assert_eq!((image.width(), image.height()), (1672, 941));
    }

    #[test]
    fn captured_backgrounds_remain_bound_to_their_wayland_output() {
        let limits = ImageLimits::default();
        let first = RgbaImage::solid(1, 1, [180, 20, 20, 255], limits).unwrap();
        let second = RgbaImage::solid(1, 1, [20, 20, 180, 255], limits).unwrap();
        let captured = HashMap::from([(7, first), (11, second)]);

        assert_eq!(
            background_for_output(&captured, None, Some(7))
                .unwrap()
                .pixels(),
            &[180, 20, 20, 255]
        );
        assert_eq!(
            background_for_output(&captured, None, Some(11))
                .unwrap()
                .pixels(),
            &[20, 20, 180, 255]
        );
        assert!(background_for_output(&captured, None, Some(99)).is_none());
        assert!(background_for_output(&captured, None, None).is_none());
    }

    #[test]
    fn uncaptured_wayland_output_uses_only_the_shared_fallback() {
        let fallback = RgbaImage::solid(
            1,
            1,
            [LOCK_COLOR[0], LOCK_COLOR[1], LOCK_COLOR[2], 255],
            ImageLimits::default(),
        )
        .unwrap();
        let captured = HashMap::new();

        assert!(std::ptr::eq(
            background_for_output(&captured, Some(&fallback), Some(99)).unwrap(),
            &fallback
        ));
    }
}
