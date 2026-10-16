//! X11 backend with verified core keyboard and pointer grabs.
//!
//! X11 cannot provide the compositor-enforced fail-closed guarantee of
//! `ext-session-lock-v1`. This backend therefore treats every failed request
//! as a lock-acquisition failure and never claims stronger isolation.

use crate::auth::{AuthOutcome, AuthWorker, PamProvider, SubmitError};
use crate::background::{
    BackgroundReceiver, BackgroundSource, LOCK_COLOR, StaticBackground, start_loader,
};
use crate::image::{FitMode, ImageEffect, ImageLimits, RgbaImage};
use crate::indicator::{AuthIndicator, IndicatorStyle, draw_indicator};
use crate::secret::SecretBuffer;
use crate::state::{LockEvent, LockMachine, LockState};
use std::error::Error;
use std::fmt;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::image::{Image as X11Image, PixelLayout};
use x11rb::protocol::Event;
use x11rb::protocol::randr::{ConnectionExt as _, NotifyMask};
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt as _, CreateGCAux,
    CreateWindowAux, EventMask, Gcontext, GrabMode, GrabStatus, Rectangle, Screen, StackMode,
    Visualid, Window, WindowClass,
};
use x11rb::xcb_ffi::XCBConnection;
use xkbcommon::xkb::{self, KeyDirection, Keycode, Keysym};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const GRAB_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const GRAB_ATTEMPTS: usize = 6;

type PerRootImages = Vec<(Window, RgbaImage)>;
type CaptureReceiver = Receiver<Result<PerRootImages, String>>;

struct RootCapture {
    root: Window,
    pixel_layout: PixelLayout,
    image: X11Image<'static>,
}

#[derive(Debug)]
pub struct X11Error(String);

impl X11Error {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for X11Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for X11Error {}

struct LockWindow {
    root: Window,
    window: Window,
    gc: Gcontext,
    depth: u8,
    pixel_layout: PixelLayout,
    width: u16,
    height: u16,
    root_visual: Visualid,
}

enum PendingBackground {
    Shared(BackgroundReceiver),
    PerRoot(CaptureReceiver),
}

impl PendingBackground {
    fn try_recv(&self) -> Result<Result<LoadedBackground, String>, TryRecvError> {
        match self {
            Self::Shared(receiver) => receiver
                .try_recv()
                .map(|result| result.map(LoadedBackground::Shared)),
            Self::PerRoot(receiver) => receiver
                .try_recv()
                .map(|result| result.map(LoadedBackground::PerRoot)),
        }
    }
}

enum LoadedBackground {
    Shared(RgbaImage),
    PerRoot(PerRootImages),
}

impl LoadedBackground {
    fn image_for(&self, root: Window) -> Option<&RgbaImage> {
        match self {
            Self::Shared(image) => Some(image),
            Self::PerRoot(images) => images
                .iter()
                .find_map(|(candidate, image)| (*candidate == root).then_some(image)),
        }
    }
}

/// Lock an X11 display until PAM authentication succeeds.
pub fn run(
    pam_service: &str,
    username: &str,
    static_background: Option<StaticBackground>,
) -> Result<(), X11Error> {
    let (connection, _preferred_screen) = XCBConnection::connect(None)
        .map_err(|error| X11Error::new(format!("could not connect to X11: {error}")))?;
    require_randr(&connection)?;
    let limits = ImageLimits::default();
    let (capture_effect, mut pending_background, background_fit) = match static_background {
        Some(StaticBackground {
            source: BackgroundSource::DesktopCapture,
            fit,
            effect,
        }) => (Some(effect), None, fit),
        background => {
            let (receiver, fit) = start_loader(background, limits);
            (None, receiver.map(PendingBackground::Shared), fit)
        }
    };
    let mut loaded_background = None;
    let mut keyboard = XkbKeyboard::new(&connection)?;
    let mut machine = LockMachine::new();
    let mut auth = AuthWorker::spawn(PamProvider::new(pam_service, username)).map_err(|error| {
        X11Error::new(format!("could not create authentication worker: {error}"))
    })?;
    let mut secret = SecretBuffer::standard()
        .map_err(|error| X11Error::new(format!("could not allocate password buffer: {error}")))?;
    if !secret.is_memory_locked() {
        eprintln!("amorphis: warning: password buffer could not be locked against swap");
    }
    let mut active_attempt = None;
    let mut indicator = AuthIndicator::default();
    let mut windows = create_lock_windows(&connection)?;
    let cursor = create_invisible_cursor(&connection, windows[0].root)?;

    machine
        .apply(LockEvent::RequestLock)
        .map_err(|error| X11Error::new(error.to_string()))?;
    acquire_input_grabs(&connection, windows[0].root, cursor)?;

    connection
        .grab_server()
        .map_err(|error| X11Error::new(format!("could not freeze X11 setup: {error}")))?
        .check()
        .map_err(|error| X11Error::new(format!("could not freeze X11 setup: {error}")))?;
    let capture_result = capture_effect.map(|_| capture_roots(&connection, &windows));
    let map_result = map_and_raise_all(&connection, &windows);
    let ungrab_result = connection
        .ungrab_server()
        .map_err(|error| X11Error::new(format!("could not unfreeze X11 setup: {error}")))?
        .check()
        .map_err(|error| X11Error::new(format!("could not unfreeze X11 setup: {error}")));
    map_result?;
    ungrab_result?;
    machine
        .apply(LockEvent::LockConfirmed)
        .map_err(|error| X11Error::new(error.to_string()))?;

    if let Some(effect) = capture_effect {
        pending_background = match capture_result {
            Some(Ok(captures)) => {
                start_capture_processor(captures, effect, limits).map(PendingBackground::PerRoot)
            }
            Some(Err(error)) => {
                eprintln!("amorphis: {error}; using the embedded fallback background");
                let fallback = StaticBackground {
                    source: BackgroundSource::EmbeddedFallback,
                    fit: background_fit,
                    effect,
                };
                start_loader(Some(fallback), limits)
                    .0
                    .map(PendingBackground::Shared)
            }
            None => None,
        };
    }

    let mut authentication_available = true;
    loop {
        while let Some(event) = connection.poll_for_event().map_err(|error| {
            X11Error::new(format!("X11 connection failed while locked: {error}"))
        })? {
            match event {
                Event::KeyPress(event) => {
                    let keycode = Keycode::new(u32::from(event.detail));
                    if machine.state() == LockState::Locked {
                        let symbol = keyboard.state.key_get_one_sym(keycode);
                        let text = keyboard.state.key_get_utf8(keycode);
                        if let Err(error) = handle_key(
                            symbol,
                            &text,
                            keyboard.control_is_active(),
                            &mut secret,
                            &mut auth,
                            &mut active_attempt,
                            &mut machine,
                            authentication_available,
                            &mut indicator,
                        ) {
                            eprintln!("amorphis: {error}; X11 remains locked");
                        }
                    }
                    keyboard.state.update_key(keycode, KeyDirection::Down);
                }
                Event::KeyRelease(event) => {
                    keyboard
                        .state
                        .update_key(Keycode::new(u32::from(event.detail)), KeyDirection::Up);
                }
                Event::MappingNotify(_) => match XkbKeyboard::new(&connection) {
                    Ok(updated) => keyboard = updated,
                    Err(error) => {
                        eprintln!("amorphis: {error}; retaining previous XKB state")
                    }
                },
                Event::RandrScreenChangeNotify(event) => {
                    if let Some(lock) = windows.iter_mut().find(|lock| lock.root == event.root) {
                        if let Err(error) =
                            resize_lock_window(&connection, lock.window, event.width, event.height)
                        {
                            eprintln!("amorphis: {error}; X11 remains locked");
                        } else {
                            lock.width = event.width;
                            lock.height = event.height;
                            let image = loaded_background.as_ref().and_then(
                                |background: &LoadedBackground| background.image_for(lock.root),
                            );
                            if let Err(error) = render_background(
                                &connection,
                                lock,
                                image,
                                background_fit,
                                limits,
                                indicator.style(),
                            ) {
                                eprintln!("amorphis: {error}; retaining solid X11 background");
                            }
                        }
                    }
                }
                Event::Expose(event) => {
                    if let Some(lock) = windows.iter().find(|lock| lock.window == event.window) {
                        let image =
                            loaded_background
                                .as_ref()
                                .and_then(|background: &LoadedBackground| {
                                    background.image_for(lock.root)
                                });
                        if let Err(error) = render_background(
                            &connection,
                            lock,
                            image,
                            background_fit,
                            limits,
                            indicator.style(),
                        ) {
                            eprintln!("amorphis: {error}; retaining solid X11 background");
                        }
                    }
                }
                Event::MapNotify(event) if !is_lock_window(event.window, &windows) => {
                    if let Err(error) = map_and_raise_all(&connection, &windows) {
                        eprintln!("amorphis: {error}; X11 remains locked");
                    }
                }
                Event::ConfigureNotify(event) if !is_lock_window(event.window, &windows) => {
                    if let Err(error) = map_and_raise_all(&connection, &windows) {
                        eprintln!("amorphis: {error}; X11 remains locked");
                    }
                }
                _ => {}
            }
        }

        let background_result = pending_background.as_ref().map(PendingBackground::try_recv);
        match background_result {
            Some(Ok(Ok(background))) => {
                pending_background = None;
                loaded_background = Some(background);
                if let Some(background) = loaded_background.as_ref() {
                    for lock in &windows {
                        if let Some(image) = background.image_for(lock.root)
                            && let Err(error) = render_background(
                                &connection,
                                lock,
                                Some(image),
                                background_fit,
                                limits,
                                indicator.style(),
                            )
                        {
                            eprintln!("amorphis: {error}; retaining solid X11 background");
                        }
                    }
                }
            }
            Some(Ok(Err(error))) => {
                pending_background = None;
                eprintln!("amorphis: {error}; retaining solid X11 background");
            }
            Some(Err(TryRecvError::Disconnected)) => {
                pending_background = None;
                eprintln!("amorphis: background worker stopped; retaining solid X11 background");
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }

        match auth.try_response() {
            Ok(Some(response)) => {
                if active_attempt.take() != Some(response.attempt) {
                    eprintln!(
                        "amorphis: ignored an uncorrelated authentication response; X11 remains locked"
                    );
                    if machine.state() == LockState::Authenticating {
                        let _ = machine.apply(LockEvent::AuthenticationFailed);
                    }
                    indicator.failed();
                    continue;
                }
                match response.outcome {
                    AuthOutcome::Success => {
                        if machine.apply(LockEvent::AuthenticationSucceeded).is_err()
                            || machine.apply(LockEvent::BeginUnlock).is_err()
                        {
                            eprintln!(
                                "amorphis: refused an invalid unlock transition; X11 remains locked"
                            );
                            continue;
                        }
                        release_input_grabs(&connection)?;
                        machine
                            .apply(LockEvent::UnlockCompleted)
                            .map_err(|error| X11Error::new(error.to_string()))?;
                        return Ok(());
                    }
                    AuthOutcome::Denied { .. } | AuthOutcome::Error { .. } => {
                        if machine.apply(LockEvent::AuthenticationFailed).is_err() {
                            eprintln!("amorphis: invalid authentication state; X11 remains locked");
                        }
                        indicator.failed();
                    }
                }
            }
            Ok(None) => {}
            Err(_) if authentication_available => {
                authentication_available = false;
                if machine.state() == LockState::Authenticating {
                    machine
                        .apply(LockEvent::AuthenticationFailed)
                        .map_err(|error| X11Error::new(error.to_string()))?;
                    active_attempt = None;
                }
                indicator.failed();
                eprintln!(
                    "amorphis: authentication worker stopped; X11 remains locked (external recovery required)"
                );
            }
            Err(_) => {}
        }

        if let Some(style) = indicator.take_redraw() {
            for lock in &windows {
                let image = loaded_background
                    .as_ref()
                    .and_then(|background| background.image_for(lock.root));
                if let Err(error) =
                    render_background(&connection, lock, image, background_fit, limits, style)
                {
                    eprintln!("amorphis: {error}; retaining previous X11 frame");
                }
            }
        }

        thread::sleep(EVENT_POLL_INTERVAL);
    }
}

struct XkbKeyboard {
    _context: xkb::Context,
    _keymap: xkb::Keymap,
    state: xkb::State,
}

impl XkbKeyboard {
    fn new(connection: &XCBConnection) -> Result<Self, X11Error> {
        let mut major = 0;
        let mut minor = 0;
        let mut event = 0;
        let mut error = 0;
        if !xkb::x11::setup_xkb_extension(
            connection,
            xkb::x11::MIN_MAJOR_XKB_VERSION,
            xkb::x11::MIN_MINOR_XKB_VERSION,
            xkb::x11::SetupXkbExtensionFlags::NoFlags,
            &mut major,
            &mut minor,
            &mut event,
            &mut error,
        ) {
            return Err(X11Error::new("XKB extension is unavailable"));
        }
        let device = xkb::x11::get_core_keyboard_device_id(connection);
        if device < 0 {
            return Err(X11Error::new("XKB core keyboard is unavailable"));
        }
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::x11::keymap_new_from_device(
            &context,
            connection,
            device,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        );
        if keymap.get_raw_ptr().is_null() {
            return Err(X11Error::new("could not read the XKB keymap"));
        }
        let state = xkb::x11::state_new_from_device(&keymap, connection, device);
        if state.get_raw_ptr().is_null() {
            return Err(X11Error::new("could not create XKB keyboard state"));
        }
        Ok(Self {
            _context: context,
            _keymap: keymap,
            state,
        })
    }

    fn control_is_active(&self) -> bool {
        self.state
            .mod_name_is_active(xkb::MOD_NAME_CTRL, xkb::STATE_MODS_EFFECTIVE)
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_key(
    symbol: Keysym,
    text: &str,
    control: bool,
    secret: &mut SecretBuffer,
    auth: &mut AuthWorker,
    active_attempt: &mut Option<u64>,
    machine: &mut LockMachine,
    authentication_available: bool,
    indicator: &mut AuthIndicator,
) -> Result<(), X11Error> {
    match symbol {
        Keysym::Escape => {
            secret.clear();
            indicator.input();
        }
        Keysym::BackSpace => {
            secret.pop_char();
            indicator.input();
        }
        Keysym::Return | Keysym::KP_Enter if authentication_available => {
            let replacement = SecretBuffer::standard().map_err(|error| {
                X11Error::new(format!("could not allocate password buffer: {error}"))
            })?;
            if !replacement.is_memory_locked() {
                eprintln!(
                    "amorphis: warning: replacement password buffer could not be locked against swap"
                );
            }
            let submitted = std::mem::replace(secret, replacement);
            match auth.submit(submitted) {
                Ok(attempt) => {
                    machine
                        .apply(LockEvent::BeginAuthentication)
                        .map_err(|error| X11Error::new(error.to_string()))?;
                    *active_attempt = Some(attempt);
                    indicator.verifying();
                }
                Err(SubmitError::Busy(value)) => *secret = value,
                Err(SubmitError::Stopped(value)) => {
                    *secret = value;
                    eprintln!(
                        "amorphis: authentication worker stopped; X11 remains locked (external recovery required)"
                    );
                }
            }
        }
        Keysym::u if control => {
            secret.clear();
            indicator.input();
        }
        _ if !control
            && !text.is_empty()
            && text.chars().all(|character| !character.is_control())
            && secret.push_str(text).is_ok() =>
        {
            indicator.input();
        }
        _ => {}
    }
    Ok(())
}

fn require_randr(connection: &XCBConnection) -> Result<(), X11Error> {
    connection
        .randr_query_version(1, 2)
        .map_err(|error| X11Error::new(format!("could not query RandR: {error}")))?
        .reply()
        .map_err(|error| X11Error::new(format!("RandR 1.2 is required: {error}")))?;
    Ok(())
}

fn create_lock_windows(connection: &XCBConnection) -> Result<Vec<LockWindow>, X11Error> {
    if connection.setup().roots.is_empty() {
        return Err(X11Error::new("X11 server has no screens"));
    }
    let mut windows = Vec::with_capacity(connection.setup().roots.len());
    for screen in &connection.setup().roots {
        let pixel_layout = root_pixel_layout(screen)?;
        let color = connection
            .alloc_color(screen.default_colormap, 0x1212, 0x1212, 0x1414)
            .map_err(|error| X11Error::new(format!("could not allocate X11 lock colour: {error}")))?
            .reply()
            .map_err(|error| X11Error::new(format!("could not allocate X11 lock colour: {error}")))?
            .pixel;
        let window = connection
            .generate_id()
            .map_err(|error| X11Error::new(format!("could not allocate X11 window id: {error}")))?;
        connection
            .create_window(
                screen.root_depth,
                window,
                screen.root,
                0,
                0,
                screen.width_in_pixels,
                screen.height_in_pixels,
                0,
                WindowClass::INPUT_OUTPUT,
                screen.root_visual,
                &CreateWindowAux::new()
                    .override_redirect(1)
                    .background_pixel(color)
                    .event_mask(
                        EventMask::EXPOSURE | EventMask::KEY_PRESS | EventMask::KEY_RELEASE,
                    ),
            )
            .map_err(|error| X11Error::new(format!("could not create X11 lock window: {error}")))?
            .check()
            .map_err(|error| X11Error::new(format!("could not create X11 lock window: {error}")))?;
        let gc = connection.generate_id().map_err(|error| {
            X11Error::new(format!("could not allocate X11 graphics id: {error}"))
        })?;
        connection
            .create_gc(gc, window, &CreateGCAux::new().graphics_exposures(0))
            .map_err(|error| {
                X11Error::new(format!("could not create X11 graphics context: {error}"))
            })?
            .check()
            .map_err(|error| {
                X11Error::new(format!("could not create X11 graphics context: {error}"))
            })?;
        connection
            .change_window_attributes(
                screen.root,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
            )
            .map_err(|error| X11Error::new(format!("could not watch X11 root window: {error}")))?
            .check()
            .map_err(|error| X11Error::new(format!("could not watch X11 root window: {error}")))?;
        connection
            .randr_select_input(screen.root, NotifyMask::SCREEN_CHANGE)
            .map_err(|error| X11Error::new(format!("could not watch X11 screen changes: {error}")))?
            .check()
            .map_err(|error| {
                X11Error::new(format!("could not watch X11 screen changes: {error}"))
            })?;
        windows.push(LockWindow {
            root: screen.root,
            window,
            gc,
            depth: screen.root_depth,
            pixel_layout,
            width: screen.width_in_pixels,
            height: screen.height_in_pixels,
            root_visual: screen.root_visual,
        });
    }
    Ok(windows)
}

fn root_pixel_layout(screen: &Screen) -> Result<PixelLayout, X11Error> {
    let visual = screen
        .allowed_depths
        .iter()
        .find_map(|depth| {
            depth
                .visuals
                .iter()
                .find(|visual| visual.visual_id == screen.root_visual)
                .map(|visual| (depth.depth, *visual))
        })
        .ok_or_else(|| X11Error::new("could not describe the X11 root visual"))?;
    if visual.0 != screen.root_depth {
        return Err(X11Error::new("X11 root visual has an inconsistent depth"));
    }
    PixelLayout::from_visual_type(visual.1)
        .map_err(|error| X11Error::new(format!("unsupported X11 root visual: {error}")))
}

fn capture_roots(
    connection: &XCBConnection,
    windows: &[LockWindow],
) -> Result<Vec<RootCapture>, X11Error> {
    let mut captures = Vec::with_capacity(windows.len());
    for lock in windows {
        let (captured, visual): (X11Image<'static>, Visualid) =
            X11Image::get(connection, lock.root, 0, 0, lock.width, lock.height)
                .map_err(|error| X11Error::new(format!("could not capture X11 root: {error}")))?;
        if visual != lock.root_visual || captured.depth() != lock.depth {
            return Err(X11Error::new(
                "captured X11 root does not match its configured visual",
            ));
        }
        captures.push(RootCapture {
            root: lock.root,
            pixel_layout: lock.pixel_layout,
            image: captured,
        });
    }
    Ok(captures)
}

fn capture_to_rgba(
    capture: RootCapture,
    limits: ImageLimits,
) -> Result<(Window, RgbaImage), String> {
    let width = capture.image.width();
    let height = capture.image.height();
    let mut rgba = RgbaImage::solid(u32::from(width), u32::from(height), [0, 0, 0, 255], limits)
        .map_err(|error| format!("could not allocate X11 capture: {error}"))?;
    let row_width = usize::from(width);
    for y in 0..height {
        for x in 0..width {
            let (red, green, blue) = capture.pixel_layout.decode(capture.image.get_pixel(x, y));
            let offset = (usize::from(y) * row_width + usize::from(x)) * 4;
            rgba.pixels_mut()[offset..offset + 4].copy_from_slice(&[
                component_to_u8(red),
                component_to_u8(green),
                component_to_u8(blue),
                255,
            ]);
        }
    }
    Ok((capture.root, rgba))
}

fn component_to_u8(value: u16) -> u8 {
    ((u32::from(value) + 128) / 257) as u8
}

fn start_capture_processor(
    captures: Vec<RootCapture>,
    effect: ImageEffect,
    limits: ImageLimits,
) -> Option<CaptureReceiver> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let spawn = thread::Builder::new()
        .name("amorphis-capture".into())
        .spawn(move || {
            let result = captures
                .into_iter()
                .map(|capture| {
                    let (root, mut image) = capture_to_rgba(capture, limits)?;
                    effect
                        .apply(&mut image, limits)
                        .map_err(|error| format!("could not apply capture effect: {error}"))?;
                    Ok((root, image))
                })
                .collect();
            let _ = sender.send(result);
        });
    if let Err(error) = spawn {
        eprintln!(
            "amorphis: could not start capture processor: {error}; retaining solid background"
        );
        return None;
    }
    Some(receiver)
}

fn render_background(
    connection: &XCBConnection,
    lock: &LockWindow,
    source: Option<&RgbaImage>,
    fit: FitMode,
    limits: ImageLimits,
    indicator: IndicatorStyle,
) -> Result<(), X11Error> {
    let mut fitted = match source {
        Some(source) => source.fitted(
            u32::from(lock.width),
            u32::from(lock.height),
            fit,
            LOCK_COLOR,
            limits,
        ),
        None => RgbaImage::solid(
            u32::from(lock.width),
            u32::from(lock.height),
            LOCK_COLOR,
            limits,
        ),
    }
    .map_err(|error| X11Error::new(format!("could not fit X11 background: {error}")))?;
    draw_indicator(&mut fitted, indicator);
    let mut image =
        X11Image::allocate_native(lock.width, lock.height, lock.depth, connection.setup())
            .map_err(|error| X11Error::new(format!("unsupported X11 image format: {error}")))?;

    for (index, rgba) in fitted.pixels().as_chunks::<4>().0.iter().enumerate() {
        let [red, green, blue] = composite_rgb(*rgba, LOCK_COLOR);
        let pixel = lock.pixel_layout.encode((
            u16::from(red) * 257,
            u16::from(green) * 257,
            u16::from(blue) * 257,
        ));
        let x = u16::try_from(index % usize::from(lock.width))
            .map_err(|_| X11Error::new("X11 image x coordinate overflowed"))?;
        let y = u16::try_from(index / usize::from(lock.width))
            .map_err(|_| X11Error::new("X11 image y coordinate overflowed"))?;
        image.put_pixel(x, y, pixel);
    }

    let requests = image
        .put(connection, lock.window, lock.gc, 0, 0)
        .map_err(|error| X11Error::new(format!("could not draw X11 background: {error}")))?;
    for request in requests {
        request
            .check()
            .map_err(|error| X11Error::new(format!("could not draw X11 background: {error}")))?;
    }
    connection
        .flush()
        .map_err(|error| X11Error::new(format!("could not flush X11 background: {error}")))?;
    Ok(())
}

fn composite_rgb(source: [u8; 4], background: [u8; 4]) -> [u8; 3] {
    let alpha = u32::from(source[3]);
    let inverse = 255 - alpha;
    let mut opaque = [0_u8; 3];
    for channel in 0..3 {
        opaque[channel] =
            ((u32::from(source[channel]) * alpha + u32::from(background[channel]) * inverse + 127)
                / 255) as u8;
    }
    opaque
}

fn create_invisible_cursor(connection: &XCBConnection, root: Window) -> Result<u32, X11Error> {
    let pixmap = connection
        .generate_id()
        .map_err(|error| X11Error::new(format!("could not allocate X11 pixmap id: {error}")))?;
    let gc = connection
        .generate_id()
        .map_err(|error| X11Error::new(format!("could not allocate X11 graphics id: {error}")))?;
    let cursor = connection
        .generate_id()
        .map_err(|error| X11Error::new(format!("could not allocate X11 cursor id: {error}")))?;
    connection
        .create_pixmap(1, pixmap, root, 1, 1)
        .map_err(|error| X11Error::new(format!("could not create X11 cursor pixmap: {error}")))?
        .check()
        .map_err(|error| X11Error::new(format!("could not create X11 cursor pixmap: {error}")))?;
    connection
        .create_gc(gc, pixmap, &CreateGCAux::new().foreground(0))
        .map_err(|error| {
            X11Error::new(format!(
                "could not create X11 cursor graphics context: {error}"
            ))
        })?
        .check()
        .map_err(|error| {
            X11Error::new(format!(
                "could not create X11 cursor graphics context: {error}"
            ))
        })?;
    connection
        .poly_fill_rectangle(
            pixmap,
            gc,
            &[Rectangle {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }],
        )
        .map_err(|error| X11Error::new(format!("could not clear X11 cursor pixmap: {error}")))?
        .check()
        .map_err(|error| X11Error::new(format!("could not clear X11 cursor pixmap: {error}")))?;
    connection
        .create_cursor(cursor, pixmap, pixmap, 0, 0, 0, 0, 0, 0, 0, 0)
        .map_err(|error| X11Error::new(format!("could not create invisible X11 cursor: {error}")))?
        .check()
        .map_err(|error| {
            X11Error::new(format!("could not create invisible X11 cursor: {error}"))
        })?;
    connection.free_gc(gc).map_err(|error| {
        X11Error::new(format!("could not release X11 graphics context: {error}"))
    })?;
    connection
        .free_pixmap(pixmap)
        .map_err(|error| X11Error::new(format!("could not release X11 cursor pixmap: {error}")))?;
    Ok(cursor)
}

fn acquire_input_grabs(
    connection: &XCBConnection,
    root: Window,
    cursor: u32,
) -> Result<(), X11Error> {
    let mut final_pointer = GrabStatus::ALREADY_GRABBED;
    let mut final_keyboard = GrabStatus::ALREADY_GRABBED;

    for attempt in 0..GRAB_ATTEMPTS {
        let pointer = connection
            .grab_pointer(
                false,
                root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                cursor,
                CURRENT_TIME,
            )
            .map_err(|error| X11Error::new(format!("could not request X11 pointer grab: {error}")))?
            .reply()
            .map_err(|error| X11Error::new(format!("could not request X11 pointer grab: {error}")))?
            .status;
        let keyboard = connection
            .grab_keyboard(false, root, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)
            .map_err(|error| {
                X11Error::new(format!("could not request X11 keyboard grab: {error}"))
            })?
            .reply()
            .map_err(|error| {
                X11Error::new(format!("could not request X11 keyboard grab: {error}"))
            })?
            .status;
        final_pointer = pointer;
        final_keyboard = keyboard;

        if pointer == GrabStatus::SUCCESS && keyboard == GrabStatus::SUCCESS {
            return Ok(());
        }
        if pointer == GrabStatus::SUCCESS {
            connection.ungrab_pointer(CURRENT_TIME).map_err(|error| {
                X11Error::new(format!("could not undo partial X11 pointer grab: {error}"))
            })?;
        }
        if keyboard == GrabStatus::SUCCESS {
            connection.ungrab_keyboard(CURRENT_TIME).map_err(|error| {
                X11Error::new(format!("could not undo partial X11 keyboard grab: {error}"))
            })?;
        }
        roundtrip(connection)?;

        if (pointer != GrabStatus::SUCCESS && pointer != GrabStatus::ALREADY_GRABBED)
            || (keyboard != GrabStatus::SUCCESS && keyboard != GrabStatus::ALREADY_GRABBED)
        {
            break;
        }
        if attempt + 1 < GRAB_ATTEMPTS {
            thread::sleep(GRAB_RETRY_INTERVAL);
        }
    }

    Err(X11Error::new(format!(
        "input grabs failed (pointer {final_pointer:?}, keyboard {final_keyboard:?}); session unchanged"
    )))
}

fn resize_lock_window(
    connection: &XCBConnection,
    window: Window,
    width: u16,
    height: u16,
) -> Result<(), X11Error> {
    connection
        .configure_window(
            window,
            &ConfigureWindowAux::new()
                .width(u32::from(width))
                .height(u32::from(height))
                .stack_mode(StackMode::ABOVE),
        )
        .map_err(|error| X11Error::new(format!("could not resize X11 lock window: {error}")))?
        .check()
        .map_err(|error| X11Error::new(format!("could not resize X11 lock window: {error}")))?;
    connection
        .clear_area(false, window, 0, 0, 0, 0)
        .map_err(|error| X11Error::new(format!("could not repaint X11 lock window: {error}")))?
        .check()
        .map_err(|error| X11Error::new(format!("could not repaint X11 lock window: {error}")))?;
    Ok(())
}

fn map_and_raise_all(connection: &XCBConnection, windows: &[LockWindow]) -> Result<(), X11Error> {
    for lock in windows {
        connection
            .map_window(lock.window)
            .map_err(|error| X11Error::new(format!("could not map X11 lock window: {error}")))?
            .check()
            .map_err(|error| X11Error::new(format!("could not map X11 lock window: {error}")))?;
        connection
            .configure_window(
                lock.window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            )
            .map_err(|error| X11Error::new(format!("could not raise X11 lock window: {error}")))?
            .check()
            .map_err(|error| X11Error::new(format!("could not raise X11 lock window: {error}")))?;
    }
    connection
        .flush()
        .map_err(|error| X11Error::new(format!("could not flush X11 lock windows: {error}")))?;
    Ok(())
}

fn release_input_grabs(connection: &XCBConnection) -> Result<(), X11Error> {
    connection
        .ungrab_keyboard(CURRENT_TIME)
        .map_err(|error| X11Error::new(format!("could not release X11 keyboard grab: {error}")))?
        .check()
        .map_err(|error| X11Error::new(format!("could not release X11 keyboard grab: {error}")))?;
    connection
        .ungrab_pointer(CURRENT_TIME)
        .map_err(|error| X11Error::new(format!("could not release X11 pointer grab: {error}")))?
        .check()
        .map_err(|error| X11Error::new(format!("could not release X11 pointer grab: {error}")))?;
    roundtrip(connection)
}

fn roundtrip(connection: &XCBConnection) -> Result<(), X11Error> {
    connection
        .get_input_focus()
        .map_err(|error| X11Error::new(format!("X11 roundtrip failed: {error}")))?
        .reply()
        .map_err(|error| X11Error::new(format!("X11 roundtrip failed: {error}")))?;
    Ok(())
}

fn is_lock_window(window: Window, windows: &[LockWindow]) -> bool {
    windows.iter().any(|lock| lock.window == window)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_client2::ErrorCode;

    struct NeverCalledProvider;

    impl crate::auth::AuthProvider for NeverCalledProvider {
        fn authenticate(&mut self, _secret: SecretBuffer) -> AuthOutcome {
            AuthOutcome::Denied {
                stage: crate::auth::AuthStage::Authenticate,
                code: ErrorCode::AUTH_ERR,
            }
        }
    }

    #[test]
    fn editing_keys_do_not_bypass_lock_state() {
        let mut auth = AuthWorker::spawn(NeverCalledProvider).unwrap();
        let mut secret = SecretBuffer::new(16).unwrap();
        secret.push_str("abc").unwrap();
        let mut attempt = None;
        let mut machine = LockMachine::new();
        machine.apply(LockEvent::RequestLock).unwrap();
        machine.apply(LockEvent::LockConfirmed).unwrap();
        let mut indicator = AuthIndicator::default();

        handle_key(
            Keysym::BackSpace,
            "",
            false,
            &mut secret,
            &mut auth,
            &mut attempt,
            &mut machine,
            true,
            &mut indicator,
        )
        .unwrap();
        assert_eq!(secret.as_bytes(), b"ab");
        assert_eq!(machine.state(), LockState::Locked);
        assert_eq!(indicator.style(), IndicatorStyle::Input);
    }

    #[test]
    fn transparent_pixels_are_composited_over_the_opaque_lock_colour() {
        assert_eq!(
            composite_rgb([200, 100, 50, 0], [10, 20, 30, 255]),
            [10, 20, 30]
        );
        assert_eq!(
            composite_rgb([200, 100, 50, 255], [10, 20, 30, 255]),
            [200, 100, 50]
        );
    }

    #[test]
    fn expanded_x11_components_convert_back_to_eight_bits() {
        assert_eq!(component_to_u8(0), 0);
        assert_eq!(component_to_u8(0x8080), 128);
        assert_eq!(component_to_u8(u16::MAX), 255);
    }

    #[test]
    fn captured_backgrounds_remain_bound_to_their_x11_root() {
        let limits = ImageLimits::default();
        let first = RgbaImage::solid(1, 1, [180, 20, 20, 255], limits).unwrap();
        let second = RgbaImage::solid(1, 1, [20, 20, 180, 255], limits).unwrap();
        let backgrounds = LoadedBackground::PerRoot(vec![(7, first), (11, second)]);

        assert_eq!(
            backgrounds.image_for(7).unwrap().pixels(),
            &[180, 20, 20, 255]
        );
        assert_eq!(
            backgrounds.image_for(11).unwrap().pixels(),
            &[20, 20, 180, 255]
        );
        assert!(backgrounds.image_for(99).is_none());
    }
}
