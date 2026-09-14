# Findings and resolutions

- The first audio authoring attempt found that the default FFmpeg build lacks
  `libvorbis`. The already-installed `ffmpeg-full` build provides it. The builder
  now accepts an explicit executable path; no package was installed or production
  dependency added. All sixteen final cues decode successfully.
- The first all-target test invocation raced the parallel agent writing its new
  test module. Rust reported the missing module; the completed source was then
  verified in a fresh workspace run. That setup failure is not final evidence.
- Source review tightened audio teardown ordering after the network session
  lifecycle and added a headless one-music-voice/pause/resume test. A focused client
  rerun and subsequent native build verify the current source, after the workspace
  regression. The source was frozen before these final checks.

Native verification and remaining limits are recorded in evidence.md/json.
No physical-device or subjective-listening acceptance is claimed.

- First native run passed actual sink/volume/mute/persistence checks in all three
  modes. Visual review found the mobile mute button needed a small initial scroll;
  reducing settings row gaps places the entire audio section on the initial phone
  view. The old technical preference hint was replaced with a concise automatic-save
  message. Final captures are rebuilt after these UI refinements.
- Strict Clippy requested `contains` instead of `find(...).is_none()` in a settings
  assertion; the equivalent test was simplified and rechecked.
