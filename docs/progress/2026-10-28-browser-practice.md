# Browser practice and portal integration

Source base: `origin/dev` 7ace97c. Task evidence:
`.agent/tasks/BROWSER-PRACTICE-20261028/`.

Added a bounded Bevy/WebGL 2 browser renderer over the existing practice
authority, with the native game's map, bots, collision, progression, towers,
skills and sprite/terrain art. The browser host keeps its own virtual simulation
epoch so backdated timers work immediately after a fresh page load. Common/shared
use one monotonic-clock boundary; native retains exactly std::time::Instant.

The portal consumes a self-contained relative-path artifact under `/game/`;
VK identity stays in its existing BFF/Account API flow. This package never uses
account credentials or grants career rewards. Browser multiplayer, publication,
provider setup and production infrastructure are outside this change.

Verification results and exact artifact hashes are recorded in the task evidence.
The package being built is distinct from it being uploaded, deployed or available
to public players. Desktop viewport evidence is not physical-mobile acceptance.
