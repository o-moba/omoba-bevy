# cpal 0.17.3 with an ALSA POLLERR fix

Unmodified [cpal 0.17.3](https://crates.io/crates/cpal/0.17.3) (Apache-2.0, see
`LICENSE`) except `src/host/alsa/mod.rs`, marked `OMOBA patch`.

**Problem.** On Linux (e.g. Fedora with PipeWire's ALSA plugin) the output PCM can
enter an xrun or suspended state. ALSA keeps `POLLERR` raised until the PCM is
recovered, but cpal 0.17 reported the error and polled again at once. The
realtime-priority audio thread then spun at 100% CPU, rodio logged
"`alsa::poll()` returned POLLERR" thousands of times per second, the game lagged
and the desktop offered to kill it.

**Patch** (backport of cpal 0.18 "polling errors trigger underrun recovery
instead of looping"):

- `POLLERR` goes through the existing underrun recovery (`prepare()`), after
  `resume()` for a suspended PCM.
- A disconnected PCM reports `DeviceNotAvailable` once and stops the worker.
- Consecutive failures back off 1, 2, 4 … 100 ms, so no failure mode can
  busy-loop the worker or flood the log.

Remove this directory and the `[patch.crates-io]` entry once `bevy_audio`
depends on rodio with cpal ≥ 0.18.
