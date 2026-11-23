# Arch Linux package

`PKGBUILD`, `.SRCINFO`, and `amorphis.pam` form the source package for the
Arch User Repository. The recipe downloads the archive from the signed
`v0.2.0` tag, checks its digest, and runs the Rust test suite. It installs
both binaries in `/usr/bin`, the examples in `/usr/share/amorphis/examples/`,
and the PAM policy as `/etc/pam.d/amorphis`. Install `playerctl` separately
to use the optional MPRIS playback rules.

Before publishing an update, build it in a clean Arch environment, run
`namcap` on both `PKGBUILD` and the resulting package, and regenerate
`.SRCINFO` with:

```sh
makepkg --printsrcinfo > .SRCINFO
```

The PAM policy is marked as a backup file, so `pacman` applies its normal
`.pacnew` handling to local changes.
