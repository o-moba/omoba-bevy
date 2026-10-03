# Mobile playtest follow-up — 2026-10-04

Version: 0.36.0; protocol 5 unchanged. Branch: `fix/mobile-playtest-0351`, based on `db0b4bd`. Task proof: `.agent/tasks/MOBILE-PLAYTEST-20261004/`.

## Implementation

- Online utilities: removed an erroneous recipe-presence guard in the client sender. Real signed transport, match/identity binding and server authority remain intact.
- Targeting/HUD: exact enemy identity portraits require alive, visible, enemy-team actors in sight range; buttons consume their pointer, disappear with stale visibility and avoid the default phone skill controls. Gold/quick-buy anchor eight logical points below the minimap. Routine no-target/direction/range attack banners are removed from all input paths.
- Practice/social: Offline Practice is always available, with an explicit local/no-history notice and a distinct server bot option. Item commands now reach local authority. Offline debug messages and free reactions use the actual local sender without waiting for a server acknowledgement. Messages, overhead bubbles, reactions and errors expire; successful acknowledgements do not occupy a persistent status line.
- Recall/FPS: cyan/gold world-space channel geometry follows the real recall timer, including cancellation and fog visibility. iOS display-link range requests the selected supported cadence; FPS measurement uses real frame time and remains honest about actual 30/60/120 rendering. A setting does not certify sustained physical-device performance.
- Lifecycle: allocated matches retain disconnected seats for a bounded three-minute grace. Terminal receipts take precedence over stale Running snapshots; PostMatch blocks underlying gameplay. Direct-server terminal replay returns Home when RequestRematch would be rejected. Fresh matchmaking cannot hand the client back into a retired allocation.
- Home: enlarge the hero by up to 35%, preserve the existing bounds fit and pedestal anchor, and separate smaller phone navigation buttons.

## Verification

Acceptance criteria AC1–AC10 passed within the defined scope. Evidence and captures: `.agent/tasks/MOBILE-PLAYTEST-20261004/evidence.md` and `evidence.json`. Rust suites pass 1394 tests (4 ignored), Python checks 200 tests, strict Clippy and formatting pass, and the native offline end-to-end flow passes. Native visuals use one English 852×393 iPhone landscape profile on macOS; they do not certify physical touch, ProMotion performance or iOS suspension. No production deployment, mobile packaging or CI changes are part of this task.


Final visual review also caught and repaired phone chat history clipping: successful send closes the keyboard, expands history, and permits touch scrolling. Only the newest overhead chat or reaction appears per hero; messages include a bounded nickname prefix. The final recall ring is larger and clearly cyan outside ordinary target markers. The Home nameplate sits below the visible feet, and the local-practice notice has a dark backing.

Final verification used synthetic focus/actions for the native gameplay harness because macOS background focus intentionally blocks phone controls. The FPS values in these captures are not a physical-device performance claim. Beta deployment and TestFlight publication remain unchanged; both client and server updates are needed for all fixes. No new production dependency, database schema or wire-format change.
