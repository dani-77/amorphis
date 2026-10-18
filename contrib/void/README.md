# Void Linux package

`template.in` is the release template for the `d77` XBPS repository. The
version and checksum remain explicit placeholders so an untagged development
snapshot cannot accidentally be published as a release.

After creating a stable `vX.Y.Z` tag:

1. download the GitHub source archive for that exact tag;
2. calculate its SHA-256 digest;
3. replace `@VERSION@` and `@CHECKSUM@` in a copy named `template`;
4. place it at `srcpkgs/amorphis/template` in `d77void-pkgs`;
5. run `./xbps-src pkg -Q amorphis` and inspect the resulting package;
6. install the package in a disposable or recoverable session before
   publishing it.

The package installs the reviewed Void PAM policy as `/etc/pam.d/amorphis` and
marks it as a configuration file, so a locally modified policy is preserved by
XBPS upgrades.
