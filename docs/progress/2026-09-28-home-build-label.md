# Home build identification

Added `client/src/build_info.rs` to format the compiled version/configuration and optional existing Xcode receipt; no build script, signing or deployment changes. `frontend/home.rs` places the noninteractive label below the Home navigation/footer on mobile/desktop. Patch version 0.28.1, workspace lock entries and docs updated.

Validation: formatting, diff and metadata/source review only. Two focused Rust tests cover valid/modified packaged identity and graceful missing/malformed/mismatched receipt fallback, but are not executed in this session. The disk has 18 GiB available, below the repository build threshold, so Cargo compilation, native layout capture and physical iPad verification are deferred. No new app has been built or installed.

Proof artifacts: `.agent/tasks/TASK-HOME-BUILD-LABEL-2026-09-28/`.
