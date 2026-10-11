# Linux batch administrator approval

The existing **Approve administrator access once per batch** setting now applies
to Linux as well as Windows. It remains enabled by default. At least two eligible
native installer operations are required; an explicitly selected version uses the
normal individual installation workflow. Portable operations remain unelevated.

Linux uses PolicyKit (`pkexec`) to start a temporary, headless package helper.
The ordinary manager downloads and verifies the packages before requesting
approval. A checksum pins the approved manifest. The helper authenticates the
calling account and socket peer, copies all approved packages into a root-owned
private directory, and verifies those copies before accepting package requests.
Requests identify an approved package by index; the helper does not accept shell
commands, new paths, repeated requests, or downgrade requests.

Only native package-manager commands execute as root. Application detection,
configuration writes, desktop shortcuts, portable extraction, and centralized
operation tracking continue in the unelevated manager. The existing per-app
format/architecture selections and mixed-batch execution are preserved.

Cancellation stops pending work. An already-running apt, dnf, or pacman
transaction is allowed to finish, after which the manager records its result.
Closing the connection ends the helper after the current transaction finishes.
Failures do not turn successful earlier operations into failures. There is no
persistent root service, sudo credential cache, or change to PolicyKit policy.

Run the manager from the Linux desktop so a graphical authentication agent is
available. Ubuntu's apt workflow is automated-tested without real installations;
Fedora/dnf and Arch/pacman retain their platform-specific command construction but
require manual VM validation. Actual authentication and multi-package installation
still require a manual test. No package installations are performed by unit tests.
