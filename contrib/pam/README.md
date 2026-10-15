# PAM policy for development

Amorphis deliberately refuses to request a session lock unless
`/etc/pam.d/amorphis` exists. PAM policy is distribution-specific: inspect the
equivalent login/locker policy on the target system before installing one of
these starting points.

Void Linux and Arch Linux commonly use:

```pam
auth include system-auth
account include system-auth
```

Debian-family systems commonly use:

```pam
@include common-auth
@include common-account
```

Install the selected contents as `/etc/pam.d/amorphis`, owned by root and not
writable by ordinary users. A mistake here can make the lock impossible to
open, so all early testing should happen inside a nested compositor from which
the process can be terminated externally.

The current UI supports one hidden password prompt per authentication attempt.
PAM stacks that require an additional OTP, password-change flow, or another
interactive hidden prompt fail closed until multi-prompt UI support is added.
