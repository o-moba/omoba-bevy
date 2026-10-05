# Promo trailer v2 — 2026-10-05

> Consolidation follow-up: this historical worktree report is superseded for delivery state by [the main consolidation report](2026-10-05-main-consolidation.md). Its code is included in the combined 0.42.0 candidate; original verification/device limitations below remain historical evidence.

## Goal

The owner asked for a new promo trailer: the September one shows build `0.23.0-rc.6`
and the game looks much better now. Requirements: at least Full HD at 60 fps, the
phone interface in the lead, several hero classes including Wildspark, modern
typography and motion-design elements, delivered into the video archive as v2.

## What changed in the repository

- `client/src/qa/record_qa.rs`: frame-stepped capture (`OMOBA_RECORD_STEP=1`,
  `OMOBA_RECORD_PACE`, `OMOBA_RECORD_WINDOW_AT`). Game time advances exactly one
  frame interval per rendered frame (`TimeUpdateStrategy::ManualDuration`), every
  frame is saved and `frames.jsonl` carries `index / fps`. The mode runs on a
  timer with vsync off and keeps the window above the others.
- `client/src/qa/demo_qa.rs`: the director drives the offline practice, paces acts
  on the recorder clock, supports `OMOBA_QA_SCALE`, `OMOBA_DEMO_ZOOM`,
  `OMOBA_DEMO_LANE` and `OMOBA_QA_SYNTHETIC_FOCUS`, hides the FPS readout and adds
  the `lane` and `showcase` scripts. Phone play is more like a person's: a finger
  drags the class list, the thumb follows the lane, a blocked thumb walks by a
  move order, a dropped joystick is pressed again. Fights skip abilities on
  cooldown and push a quiet lane forward. Scripted clicks no longer move the real
  pointer in synthetic-focus runs.
- `scripts/record_demo.py`: `--profile v2` (six clips, `--pace`, `--window-at`,
  `--keep-frames`), offline clips without a server process, a gapless-frame check
  and a constant-60-fps encoder with BT.709 tags. `make trailer-clips`.
- `promo/trailer-v2-2026-10-05/`: notes, manifests and the Remotion source of the edit.
  The edit shows still frames extracted by frame number (`motion/prepare.py`)
  rather than playing the clips: Remotion's video extraction repeated a
  gameplay frame now and then, found by the first verification pass.

## Why frame-stepped capture

Measured on the M4 Pro with the dev build: the game renders 60 fps without capture;
with a readback every frame the recorder gets about 35 fps at 1280×720 and 14 fps at
1920×1080. Three more things stopped long captures while the desktop was in use:
the game idles an unfocused window at one update per second, the phone HUD hides
and drops held touches while its window is unfocused, and the director's scripted
clicks moved the real pointer. Stepping time removes the speed limit; the timer,
the always-on-top parked window and synthetic focus remove the dependence on focus.

Remote heroes are still interpolated on wall-clock receipt times
(`net/interpolate.rs`). In a stepped capture one snapshot arrives per frame, so
the error is below one frame of movement; production netcode was not changed.

## Checks

See `.agent/tasks/promo-trailer-v2/` for the evidence bundle and verdict.

- `cargo fmt --all --check`, `cargo clippy -p client --all-targets -- -D warnings`
  and the same without the `qa` feature: clean.
- `cargo test -p client --lib qa::demo_qa` (5 tests) and `qa::record_qa` (2 tests),
  `python3 -m unittest scripts/test_record_demo.py` (11 tests): pass.
- Six clips recorded with `--profile v2`; each passed the gapless-frame check.
- The trailer was rendered and probed with `ffprobe`; stills were reviewed per shot;
  frame-to-frame differences of the static-phone scenes show no repeated frames.
- A first verification pass failed the records (the notes said nothing was sped up
  while the hero-select tour plays at 1.5×) and reported the repeated frames, a
  crash of `--encode-only` after the raw frames are gone and a stale capture
  method string; all four were fixed and the trailer was rendered again.

## Remaining risks

- The capture was run on macOS only. The always-on-top parked window and the
  pointer handling are written against winit's macOS behaviour.
- The footage is offline practice with bots at level 6; it is labelled so in the
  manifest and on the end card. The phone scenes are not device recordings.
- Every class scene shows the default avatar (Agnes): the director joins with
  the avatar the client selects by default. Per-class avatars need a director
  option and a new recording.
- The six clips were recorded with three successive builds of the director; a
  full `--profile v2` run with the final code was not repeated end to end.
- The session left the internal disk at about 20 GB free; the shared cargo cache A
  grew from 90 GB to 111 GB while this worktree and another agent built in it.
- The workspace gate (`make check`) was not run in full; only the client crate and
  the demo script changed, and their targeted checks are listed above.
- The trailer is not published; the website and README still link the v1 trailer.
