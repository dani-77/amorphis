# Security status

Amorphis is pre-release software and has not yet completed its compositor,
distribution, suspend/resume, or hostile-input test matrix. Do not presently
use it as the sole protection for an important session.

Before locking, Amorphis disables core dumps and, on Linux, marks the process
non-dumpable. The controlled password buffer has a fixed capacity and is erased
explicitly when cleared or dropped. Passing a password through the PAM ABI
necessarily creates copies owned by PAM and its modules; Amorphis cannot
guarantee erasure of those copies. Swap and PAM-owned copies remain known
limitations.

The Wayland backend only uses `ext-session-lock-v1`. It never substitutes a
normal fullscreen, layer-shell, or XWayland window for a real session lock.
After lock confirmation, errors never call the protocol's unlock operation;
only successful PAM authentication and account validation can reach it.

The X11 backend provides weaker guarantees inherent to X11. It uses opaque
override-redirect windows, verified keyboard and pointer grabs, RandR resize
handling, and re-raising on other top-level activity. Killing Amorphis,
switching VT, terminating/resetting the X server, or a sufficiently privileged
X11 client may bypass it depending on system policy. Unlike Wayland session
lock, client death cannot remain fail-closed.

Optional X11 desktop capture freezes the server only while copying each root
and mapping the opaque lock windows. Pixel conversion and visual effects run
after coverage in a worker. Capture failure falls back to the embedded image
and then to the opaque solid background without cancelling the lock.
