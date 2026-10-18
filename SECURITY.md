# Security status

Amorphis is pre-release software and has not yet completed its compositor,
physical multi-monitor, repeated suspend/resume, or extended hostile-input test
matrix. One real Wayland suspend/resume cycle has retained full coverage and
rejected failed PAM attempts, but this is not yet broad regression coverage. Do
not presently use Amorphis as the sole protection for an important session.

Before locking, Amorphis disables core dumps and, on Linux, marks the process
non-dumpable. The controlled password buffer has a fixed capacity and is erased
explicitly when cleared or dropped. On Linux, each controlled allocation also
uses best-effort `mlock`; failure emits a warning but does not leave the session
unlocked. Passing a password through the PAM ABI necessarily creates copies
owned by the PAM client crate, PAM, and its modules; Amorphis cannot lock or
guarantee erasure of those copies. Those external copies, and the controlled
buffer on systems where `mlock` fails, remain swap-related limitations.

The lockfile was scanned against 1,329 RustSec advisories on 2026-10-16; no
known vulnerability was reported among its 75 crates. This is a point-in-time
result, not a substitute for repeating `cargo audit` as advisories and the
lockfile change.

The command-line parser, PNG decoder, and bounded image transformations have
completed AddressSanitizer-backed libFuzzer runs locally and in GitHub Actions
without a crash. A scheduled workflow repeats short runs weekly. This improves
hostile-input coverage but does not prove the absence of memory, resource, or
logic errors.

The project's direct `unsafe` blocks were manually reviewed on 2026-10-18.
They are limited to process hardening and Unix account lookup in `system.rs`,
plus memory locking, unlocking, and explicit erasure in `secret.rs`; every
block documents its pointer/lifetime preconditions. This review does not cover
unsafe internals of dependencies.

The Wayland backend only uses `ext-session-lock-v1`. It never substitutes a
normal fullscreen, layer-shell, or XWayland window for a real session lock.
It creates and maintains a lock surface for every advertised output; an output
connected after desktop capture is covered with the opaque solid background.
After lock confirmation, errors never call the protocol's unlock operation;
only successful PAM authentication and account validation can reach it.
Optional Wayland desktop capture uses `ext-image-copy-capture-v1` with
`ext-output-image-capture-source-v1`. It captures each advertised output before
requesting the session lock, excludes the pointer, retains no screenshot after
the process exits, and processes effects only after the lock request begins.
Unsupported protocols, formats, transforms, timeouts, and runtime failures fall
back to the embedded image and then the opaque solid background.

The Wayland path has been exercised on Wasp/wlroots, Spitfire/Smithay, and
Hyprland/Aquamarine as real login sessions, with an additional nested Spitfire
test. This is useful cross-compositor coverage, but it is not a claim of
universal Wayland compatibility.

The X11 backend provides weaker guarantees inherent to X11. It uses opaque
override-redirect windows, verified keyboard and pointer grabs, RandR resize
handling, and re-raising on other top-level activity. Every X screen/root is
covered; a normal multi-monitor RandR desktop is covered by its full root
window. Killing Amorphis,
switching VT, terminating/resetting the X server, or a sufficiently privileged
X11 client may bypass it depending on system policy. Unlike Wayland session
lock, client death cannot remain fail-closed.

Optional X11 desktop capture freezes the server only while copying each root
and mapping the opaque lock windows. Pixel conversion and visual effects run
after coverage in a worker. Capture failure falls back to the embedded image
and then to the opaque solid background without cancelling the lock.
