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

1. Set the release version in `Cargo.toml` and the matching dependency in
   `fuzz/Cargo.toml`. Refresh both lockfiles and date the entry in
   `CHANGELOG.md`.
2. Run the development checks in the README and test installation with a
   temporary `DESTDIR`. Confirm a clean tree and successful CI on `main`.
3. Create and push the signed release tag. For example, use
   `git tag -s v0.2.0 -m "Amorphis 0.2.0"`.
4. Download the exact source archive served by GitHub and calculate its
   SHA-256. Use the digest to refresh the packaging files on `main` in a
   follow-up commit. Follow the [Void](contrib/void/README.md) and
   [Arch](contrib/arch/README.md) instructions before publishing packages.
5. Publish the GitHub release using the matching changelog entry.
