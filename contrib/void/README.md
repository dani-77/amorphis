# Void Linux package

`template.in` is the parameterized release template for the `d77` XBPS
repository. Its explicit placeholders prevent an untagged development snapshot
from accidentally being published as a release. The sibling `template` is the
finalized template for the current stable tag.

After creating a stable `vX.Y.Z` tag:

1. download the GitHub source archive for that exact tag;
2. calculate its SHA-256 digest;
3. replace `@VERSION@` and `@CHECKSUM@` in a copy named `template`;
4. place it at `srcpkgs/amorphis/template` in `d77void-pkgs`;
5. run `./xbps-src pkg -Q amorphis` and inspect the resulting package;
6. install the package in a disposable or recoverable session before
   publishing it.

The `v0.2.0` archive has SHA-256 digest
`b0bdf4d1ec09b2da367fea9a1c5492783341f45de4b00265241a43fc27ab8c67`. This
recipe installs both binaries in `/usr/bin` and the three examples in
`/usr/share/amorphis/examples/`. Install `playerctl` separately to use
the optional MPRIS playback rules.

The package installs the reviewed Void PAM policy as `/etc/pam.d/amorphis` and
marks it as a configuration file, so a locally modified policy is preserved by
XBPS upgrades.
