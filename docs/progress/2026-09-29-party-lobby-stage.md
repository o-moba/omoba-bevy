# Party lobby and shared prematch stage

Owner request: make the party lobby a polished gathering space, with the local hero central, friends behind, all five members supported and heroes rotatable. Reuse this presentation for the class/avatar/lane draft, confirmation and shared countdown, with roughly 30 seconds to choose.

## Implementation

- Shared transparent 1280×720 renderer: viewer-first formation, two rear pairs, stepped octagonal plinths, separate hero pivots, independent idle clips, model grounding and downloaded-model refresh.
- Lobby: persistent painted backdrop, responsive authored canvas, primary group-play actions, separate scrolling social panel and honest presence/leader labels. The viewer's position never changes who leads the party.
- Prematch: actual own-team snapshot members and selection/lock/load state on the same stand; opponents remain separately identified. The local preview stays visible while choosing. Unconfirmed teammates have empty, labelled stands until server lock. A server-owned 30-second selection deadline preserves party gathering and asynchronous avatar admission. Early all-lock completion, shared countdown and loading barrier remain.
- Presentation fixtures use explicit opt-in QA switches and are labelled in screenshots/manifests. Real UDP tests remain separate evidence of party and prematch networking.

## Verification

Implemented in `bd4df40`, merged into main, then compact Russian wording and feature documentation corrected in `4872f24`. Root personally repeated the checks on main; the earlier restricted verifier was not used.

- Locked client/server build passed. Client library: 801 passed, one existing migration-report test ignored. Server filters: prematch 17, party 16, match-service 8, avatar admission 8 passed (filters may overlap).
- Production client/server Clippy and no-default-feature client Clippy passed with warnings denied; formatting, screen-map validation and five map tests passed.
- 40 final real-renderer captures: English desktop 1280×720, phone 844×390 and tablet 1180×820; Russian phone and Chinese tablet. All five runs verified touch rotation, social scrolling to its lower bound, required visible controls and no obscuring modal. Root visually reviewed the key compositions and corrected clipped Russian lock text before recapture.
- Real local UDP peers passed invitation→acceptance→leader bot launch→same-team seating with three allied and five enemy bots. Separate prematch checks passed early locking, automatic locking after 30.026 seconds without lock requests, the shared countdown, asset-ready gating and loading dropout/reconnect with a fresh generation.
- Final network checks used the same server binary as the final build. Only Russian client text and documentation changed after the network run.

The old UDP smoke failed its initial presence check with a fixed startup delay and one-shot handshake. The final task-local harness waits for listening, retries the handshake and retains full server logs. An exploratory all-targets Clippy run also found existing test-style lints in `preview_interaction.rs`; the production gates above pass, and this task's new QA lint was repaired.

Screenshots and review: [omoba-ui capture package](../../../omoba-ui/captures/2026-09-29-party-lobby/README.md). Full commands, hashes, acceptance mapping and verdict live in `.agent/tasks/UI-PARTY-LOBBY-2026-09-29/`.

Physical iOS/Android installation and Internet deployment were not tested. Captures use explicitly labelled synthetic rosters and desktop viewport simulations; the real networking evidence uses two external UDP peers. Existing avatar art/thumbnail differences were not redesigned.
