# RU preflight: standard Ubuntu os-release link

Date: 2026-11-19

## Finding

Ubuntu installs `/etc/os-release` as a relative symlink to
`../usr/lib/os-release`. `ru_host.py` previously passed that path to the general
safe file reader, which refuses every symlink. A supported Ubuntu 26.04 RU host
therefore looked like an unknown operating system and bootstrap stopped before
making changes.

## Change

`Host.os_release()` now resolves only that standard link target. Any other link
still produces no OS data and fails platform preflight. The general `read()`
policy is unchanged. This is deployment tooling only: game code, protocol,
dependencies and host configuration are unaffected.

## Verification

- `test_standard_os_release_symlink`: Ubuntu detection and platform preflight pass.
- `test_arbitrary_os_release_symlink_is_refused`: a nonstandard target remains refused.
- Focused run: 2/2 pass.
- Complete `scripts.test_ru_host`: 76/76 pass.
- Live read-only status on the intended RU host reports `ubuntu 26.04`.

The ignored local bundle `.agent/tasks/RU-OS-RELEASE-20261119/` retains the final
raw commands and results. These checks do not certify a server build, deployment,
multiplayer session, DNS, backups, capacity or public production readiness.
