# Releasing Amorphis

## v0.1.0 readiness

Completed:

- [x] version and release profile set in `Cargo.toml`;
- [x] Wayland tests in real Wasp, Spitfire, Hyprland, and SomeWM sessions;
- [x] physical two-output Wayland coverage and HDMI hotplug while locked;
- [x] X11 tests in a real Xorg session and disposable two-screen Xephyr;
- [x] one real Wayland suspend/resume authentication cycle;
- [x] formatting, 51 unit tests, Clippy, and release build in CI;
- [x] dependency advisory, ban, license, and source policy checks;
- [x] bounded AddressSanitizer fuzzing of CLI, PNG, and image operations;
- [x] review of the project's direct `unsafe` blocks;
- [x] successful XBPS build from the parameterized Void template.

Pending before a public package release:

- [x] make release archives publicly accessible by opening the GitHub
  repository;
- [x] create and push the signed `v0.1.0` tag;
- [x] calculate the tagged archive SHA-256 and finalize `contrib/void/template`;
- [x] build, inspect, and install the final XBPS package;
- [x] create the GitHub release from `CHANGELOG.md`.

Additional multi-monitor topologies and suspend/resume cycles remain desirable
and must stay documented as coverage limitations if they are not completed
before v0.1.0.

## Release procedure

1. Confirm a clean tree and a successful CI run on `main`.
2. Re-run the commands in the README's Development section.
3. Replace `Unreleased` in `CHANGELOG.md` with the release date and commit it.
4. Create a signed tag with `git tag -s v0.1.0 -m "Amorphis 0.1.0"` and push it.
5. Download the exact tag archive, calculate its SHA-256, and follow
   `contrib/void/README.md` to finalize and test the XBPS template.
6. Publish the GitHub release using the v0.1.0 changelog entry.
