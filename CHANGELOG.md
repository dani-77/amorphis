# Changelog

All notable changes to Amorphis will be documented in this file.

## [0.2.0] - 2026-11-23

### Added

- Run `amorphis` without arguments to detect the session and read its
  `x11.toml` or `wayland.toml` profile. Explicit `--backend` invocations
  continue to work.
- Run `amorphis-idle` to execute commands after configurable idle periods
  on X11 and Wayland. Rules can select a different locker for each session
  and run an optional command when activity returns.
- Defer idle rules during playback from selected MPRIS players, for a fixed
  duration, or for as long as a command is running. Applications can also
  request inhibition through the optional `org.freedesktop.ScreenSaver`
  service on the session bus. The last release starts a fresh idle countdown.
- Install both binaries and three sample TOML files with `make install`.
  See the [configuration guide](CONFIGURATION.md) for setup.

### Fixed

- Prefer the Wayland session when XWayland also sets `DISPLAY`.
- Refuse to lock on Linux with `NoNewPrivs` active, since the PAM policy
  may require a privileged helper to verify the password.

## [0.1.0] - 2026-10-22

### Added

- Fail-closed Wayland locking through `ext-session-lock-v1`, tested on Wasp,
  Spitfire, Hyprland, and SomeWM.
- X11 locking with verified keyboard/pointer grabs and RandR handling.
- PAM authentication with fixed-capacity secret storage, explicit erasure,
  process hardening, and best-effort memory locking.
- Embedded and custom PNG backgrounds, native per-output/root capture,
  `cover`/`contain`/`center` fitting, blur, and pixelation.
- Length-independent input, verification, and authentication-failure feedback.
- CI, dependency policy checks, scheduled fuzzing, PAM examples, an X11
  multi-screen smoke test, and a parameterized Void Linux package template.

### Security

- Unlock is reachable only after the display backend confirms the lock and PAM
  reports successful authentication and account validation.
- Image dimensions, allocations, blur scratch space, password capacity, and
  capture time are explicitly bounded.

### Fixed

- Initialize Wayland keyboard input before requesting a session lock, refuse to
  lock when no keyboard is available, and process the lock request separately
  from surface creation. This prevents keyboard-focus races seen on SomeWM
  2.1.0 while preserving compositor-specific lock-confirmation ordering.

[0.2.0]: https://github.com/dani-77/amorphis/releases/tag/v0.2.0
[0.1.0]: https://github.com/dani-77/amorphis/releases/tag/v0.1.0
