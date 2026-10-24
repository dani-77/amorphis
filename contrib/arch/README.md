# Arch Linux package

`PKGBUILD`, `.SRCINFO`, and `amorphis.pam` form the source package for the
Arch User Repository. The package builds the signed `v0.1.0` source archive,
runs the Rust test suite, installs the binary under `/usr/bin`, and installs the
PAM policy as `/etc/pam.d/amorphis`.

Before publishing an update, build it in a clean Arch environment, run
`namcap` on both `PKGBUILD` and the resulting package, and regenerate
`.SRCINFO` with:

```sh
makepkg --printsrcinfo > .SRCINFO
```

The PAM policy is marked as a backup file, so `pacman` applies its normal
`.pacnew` handling to local changes.
