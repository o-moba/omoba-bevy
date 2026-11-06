# Native frames without unlocking macOS

Use this for Bevy rendering/layout evidence when an OS-window screenshot is
black because the Mac is locked. Do not disable screen locking, use AppleScript
to unlock/focus windows, or accept the black image. The tested alternative is
`OMOBA_QA_OFFSCREEN=1`, implemented in `client/src/qa/offscreen.rs`.

The flag disables Winit before DefaultPlugins build. A virtual primary window
provides the chosen layout metrics; the production main camera and UI draw into
an image, with the main camera retained as default UI camera. Existing portrait/
party preview image targets stay separate. A60Hz schedule runner replaces the OS
event loop. `Screenshot::image` captures the framebuffer; no desktop, Window
Server screenshot permission, OS focus/unlock or power assertion is needed.

## Reuse or build

Use an already verified desktop client built with `qa` from the required
sources. A shared `target` may contain another worktree/market's binary: record
the actual binary SHA256 and build provenance. The normal client without `qa`
or an older binary does not support the mode. The helper refuses a native
summary that does not explicitly confirm offscreen rendering.

Only when a matching build is missing, follow the repository's cache/disk rules
and run the targeted build once. Example for **assigned cache B**:

```sh
export CARGO_TARGET_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/../target-b"
df -h /
OMOBA_MARKET=ru cargo build --locked -p client --features qa --bin client
```

Use the assigned market/build settings too (`OMOBA_MARKET=ru` for the verified
RU build). Never create a third/per-worktree Cargo cache or clean shared caches.
Stop heavy builds below the repository's free-space threshold. Documentation or
skill edits need no native rebuild.

## One isolated frame

From the game checkout:

```sh
python3 .claude/skills/omoba-ui-verification/scripts/capture_offscreen.py \
  --binary "$CARGO_TARGET_DIR/debug/client" \
  --scene remade-result --language ru --legal-version 0-draft \
  --output .agent/ui-proof/remade-result-unique
```

Choose a fresh output directory. The helper currently seeds an **RU synthetic
presentation fixture**, with its own account-region/settings/consent records,
muted audio and closed loopback game/account endpoints. It does not use the
owner's preferences or grant real legal/account consent. `--legal-version` must
match the built client's version: `0-draft` is the verified2026-11-06 fixture,
not a permanent/legal approval. Region and language are separate; the helper's
default language is English. Pick only the language/viewport the task requires.

Available individual scenes: `taken`, `party-restart`, `search-restart`,
`remake-menu`, `remake-prompt`, `surrender-prompt`, `remade-result`,
`surrender-result`. Default viewport is852×393 at scale1/touch layout. This list
is a menu, not an instruction to capture all scenes or a device/language matrix.
The helper downloads nothing, preserves failed attempts and stops only its own
process group on panic/deadline. Outputs must stay under the checkout's ignored
`.agent/`; do not publish the generated `state/` (it can contain fixture keys).

`--build-report <verified-json>` optionally checks its `binary_sha256` against
the chosen executable. `capture.json` records binary, source and runner hashes,
checkout/diff identity, scope and optional current Mac lock state. Recording
both hashes does not by itself prove the binary was built from those sources;
keep the matching build evidence.

## First-match guide regression

For the actual guide → dismiss → returned vote-prompt path:

```sh
python3 .claude/skills/omoba-ui-verification/scripts/capture_offscreen.py \
  --binary "$CARGO_TARGET_DIR/debug/client" \
  --scene remake-prompt --guide --language ru --legal-version 0-draft \
  --output .agent/ui-proof/remake-guide-unique
```

`--guide` enables beginner tips only in the owned fixture. It waits for a visible
Help dismiss control and modal, then uses the real Creator Loop `shot`/`press`
commands. It requires both `loop/guide-before-dismiss.png` and the final
`fair-play.png`. It does not hide the modal by changing production resources.

## Accept the actual image

Native checks still require the intended screen/root, expected localized text,
appropriate modal/help state, fitting required controls,44px vote targets and
nonblack color readback. The helper additionally checks PNG dimensions, locale
and `render_surface = offscreen image with virtual window metrics`.

`captured_pending_visual_inspection` is not final acceptance. Open every required
image and check the complete intended screen, readable text, clipping/overlap
and controls. A green geometry check can still hide content behind a guide.
After inspection preserve only accepted exact PNGs plus hashes/provenance in
the task's evidence pack; retain negative evidence separately. Do not repair,
crop, redraw or relabel a failed image as passed.

This proves the production renderer's selected synthetic presentation state.
It does not prove OS-window/focus/input, real admission/gameplay/account/privacy,
physical-phone layout, haptics, keep-awake or performance. For Creator Loop alone,
the flag also works with `OMOBA_CREATOR_LOOP_QA=<owned-dir>` and image screenshots,
but it supplies no gameplay fixture, account or real server: use the separately
authorized scenario and do not confuse it with the fair-play fixture helper.

Verified2026-11-06: real RU852×393 `remade-result` with the new application
renderer, readable Remake/no ordinary rewards/zero XP result; Mac reported
locked at launch. The bundled helper also produced inspected surrender-result,
remake-menu, surrender-prompt and real guide → dismiss → remake-prompt frames
while locked. Both guide and restored-prompt images were checked. The earlier
no-window diagnostic is not the game proof.
