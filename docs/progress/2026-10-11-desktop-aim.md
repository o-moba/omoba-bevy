# Desktop aim of held direction skills

Branch `fix/desktop-aim-follows-cursor`, from main `0c0585c`. Client only; no
protocol, catalog or gameplay change.

## Report

On a Linux desktop the owner could not steer Wildspark's ultimate (Last Spark,
`wild_rocket`, a direction skill with range 256) with the mouse: it did not fly
where the cursor pointed.

## Findings

The basic desktop flow is correct on every OS: hold the key, sweep the mouse,
release, and the skill is sent at the cursor; the lane follows every frame. The
platform layer was checked against winit 0.30.12, bevy_winit/bevy_input 0.19.1,
gilrs 0.11 and the X server and libinput sources:

- Key auto-repeat cannot release a held key on Linux: X11 and Xwayland use
  detectable auto-repeat (winit fails window creation without it) and Wayland
  repeats as `Pressed` with `repeat: true`.
- Window focus is reported reliably; keys only reach a focused window.

Reproduced with app-level tests (`client/src/combat/desktop_aim_tests.rs`, which
drive the real keyboard input system, controller sampler and combat chain):

1. The game is windowed by default (1280×720). Aiming a long-range skill toward an
   off-screen target moves the cursor out of the window; `cursor_position()` is
   then `None`, the lane disappears and the release was sent along the hero's
   facing (`forward * 10`).
2. Losing focus mid-hold (Alt+Tab, GNOME's Super/overview): Bevy synthesizes the
   key release on the next frame, nothing cancelled the hold, and the cast went
   along the hero's facing because the cursor is ignored without focus.
3. A connected controller that wakes up mid-hold (stick drift, a button blip, or
   a Linux joystick-class device: gilrs accepts any `ID_INPUT_JOYSTICK` node with
   one key and two axes) took input ownership; mouse motion never gives it back,
   so the lane froze on the hero's facing.
4. Without focus the lane and the cast disagreed (the lane ignored focus).
5. A same-frame Released+Pressed pair would release the cast early (no Linux
   backend produces it today; guarded anyway).

One Linux-only cause is outside the game: libinput's touchpad
*disable-while-typing* (GNOME default on) posts no pointer motion while a letter
key is held, so a laptop touchpad cannot steer any hold-to-aim skill. Workaround:
Settings → Mouse & Touchpad → turn off "Disable touchpad while typing"
(`gsettings set org.gnome.desktop.peripherals.touchpad disable-while-typing false`).
A tap-to-arm cast mode would avoid it in-game; that is a UX decision for later.

## Changes

- `DesktopAim` (sampled each frame before the keyboard cast) is the one desktop
  aim source: the cursor's ground point in a focused window, plus the last such
  point while a skill key is held. The lane (`draw_aim`) and the cast both read it;
  the release snapshots it, so a cast buffered through recovery goes where it was
  drawn.
- A desktop first cast with no aim is dropped with one line ("Point at the
  battlefield inside the game window to aim", en/ru/zh-Hans). A self cast takes
  the hero's position; recasts (zone detonation, riding a hook, follow-ups that
  ignore the aim) and touch/controller casts keep the old facing fallback, and a
  gated recast's "not ready" line still comes first. Two ally-pick tests that
  encoded "no cursor aims ten units ahead" now expect the line.
- Desktop QA demos point the cursor at their target for the frame a scripted skill
  key goes down.
- Focus loss (or an unfocused window) cancels a keyboard hold like a modal does:
  the window is marked unfocused a frame before Bevy's synthesized release.
- A release counts only if the key is not down again in the same frame.
- While a skill key is held, the controller sampler treats it as keyboard use, so
  a pad cannot take ownership mid-hold; the lane's branch follows the hold's
  source (touch, pad button, or keyboard).
- Standard-kits QA forces window focus for its aimed stages, as the other QA
  harnesses do.

## Verification

- `cargo test -p client --lib desktop_aim`: 14 passed (the five reproductions and
  three new regressions included).
- `cargo test -p client --lib`: 1303 passed, 1 ignored (environment-dependent).
- An independent review confirmed two issues in the first revision (aim-free
  recasts dropped without a cursor; demo QA key casts without a cursor), both fixed.
- `cargo clippy -p client --all-targets --no-deps -- -D warnings`, the same with
  `--features qa`, `cargo test -p client --lib --features qa aim_preview` (12) and
  `cargo fmt --all -- --check` pass.
- Worktrees share the cargo cache, and cargo judges a workspace crate fresh by file
  times, so a build can pick up another worktree's artifact. The final runs touched
  every source file first to force this worktree's code.
- Not verified on the owner's Linux machine; physical confirmation is pending.
