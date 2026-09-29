# Party lobby and shared prematch stage

Owner request: make the party lobby a polished gathering space, with the local hero central, friends behind, all five members supported and heroes rotatable. Reuse this presentation for the class/avatar/lane draft, confirmation and shared countdown, with roughly 30 seconds to choose.

## Implementation

- Shared transparent 1280×720 renderer: viewer-first formation, two rear pairs, stepped octagonal plinths, separate hero pivots, independent idle clips, model grounding and downloaded-model refresh.
- Lobby: persistent painted backdrop, responsive authored canvas, primary group-play actions, separate scrolling social panel and honest presence/leader labels. The viewer's position never changes who leads the party.
- Prematch: actual own-team snapshot members and selection/lock/load state on the same stand; opponents remain separately identified. A server-owned 30-second selection deadline preserves party gathering and asynchronous avatar admission. Early all-lock completion, shared countdown and loading barrier remain.
- Presentation fixtures use explicit opt-in QA switches and are labelled in screenshots/manifests. Real UDP tests remain separate evidence of party and prematch networking.

## Verification

Pending final gate and reviewed viewport captures. Physical iOS/Android device installation is not part of this desktop-renderer capture pass. Main integration and final root verification will be recorded in the task evidence.
