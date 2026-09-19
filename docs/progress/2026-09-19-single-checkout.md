# Single checkout and ignore hardening — 2026-09-19

Goal: keep one `omoba-bevy` checkout that builds desktop, iPhone and Android, and
keep generated or private files out of version control.

## Findings
- `omoba-bevy-iphone`, `omoba-bevy-mobile-beta` and `omoba-bevy-avatar-roundtrip`
  were local copies on the pre-rewrite history. Every HEAD commit exists on `main`
  under a new hash (`d054cd3`, `b88eff5`, `860dfa2`).
- `main` already carries the platform tooling: `mobile/ios` (device, simulator,
  install, TestFlight), `mobile/android/build.py`, `scripts/package_native.py`.
  The uncommitted `mobile/ios` edits in the iPhone copy were an older form of the
  same scripts (12 unique lines, all superseded by the 2026-09-16 rewrite).
- Unique content in the copies was task evidence only: `IPHONE-DEVICE-2026-09-14`
  and nine `*-2026-09-11/12` folders, plus `docs/progress/2026-09-14-iphone-device.md`.
- Six `.agent/tasks/*` folders (248 files, about 11 MB, mostly raw logs) were still
  tracked in the public repository although `.agent` is ignored.

## Changes
- Moved the ten evidence folders into `.agent/tasks/` and the progress note into
  `docs/progress/`.
- `git rm -r --cached .agent`; files remain on disk.
- `.gitignore`: signing material, `.env*`, Xcode user data and archives, Gradle
  state, packaged artifacts, virtualenvs, `node_modules`, IDE folders, `*.log`.

## Checks
- `git ls-files .agent` is empty; `git status` shows no build output as untracked.
- No process was running from the retired copies before removal.

## Remaining risk
- The untracked evidence is still present in earlier public history.
- Four tracked files under `docs/` match the older `docs/*` rule without an
  allowlist entry; they stay tracked, new siblings would be ignored.
