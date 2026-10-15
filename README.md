# Amorphis

Amorphis is intended to become a small, fail-closed screen locker with real
Wayland and X11 backends, PAM authentication, and optional blurred or
pixelated backgrounds.

## Current status

The project is under construction. It now has an early Wayland path using
`ext-session-lock-v1` and an early X11 path with verified keyboard/pointer
grabs, both sharing PAM authentication and a fail-closed state machine. It has
not yet had the real-server test matrix or security review required for
production use. Do not rely on it to protect an important session yet.

The implementation direction and security constraints are documented in:

- [`AMORPHIS_DIRECAO.md`](AMORPHIS_DIRECAO.md)
- [`AMORPHIS_PROPOSTA_TECNICA.md`](AMORPHIS_PROPOSTA_TECNICA.md)

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Before any live test, install a PAM policy named `amorphis`; examples and the
associated warning are in [`contrib/pam/README.md`](contrib/pam/README.md).
Then test in a nested compositor with a solid background:

```sh
cargo run -- --backend wayland
```

Or use a PNG, decoded under explicit dimension and memory limits in a worker
while the opaque lock frame is already being established:

```sh
cargo run -- --backend wayland --image /path/to/background.png --fit cover
```

The fit modes are `cover` (default), `contain`, and `center`. A missing, invalid,
or oversized image is non-fatal and leaves the solid background in place.

The command refuses to request a lock when `/etc/pam.d/amorphis` is absent. It
also refuses Wayland compositors without `ext-session-lock-v1`; there is no
layer-shell or XWayland fallback.

The initial solid-colour X11 backend is selected explicitly:

```sh
cargo run -- --backend x11
```

It requires XKB and RandR, creates one opaque window per X screen, and verifies
both input grabs before reporting the lock as established. X11 cannot retain a
lock if this process or the X server is killed; see [`SECURITY.md`](SECURITY.md).

