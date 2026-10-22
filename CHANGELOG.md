# Changelog

All notable changes to Amorphis will be documented in this file.

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

[0.1.0]: https://github.com/dani-77/amorphis/releases/tag/v0.1.0
