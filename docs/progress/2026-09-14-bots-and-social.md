# Bot practice and match communication — 2026-09-14

Implementation base: `8872215`. Delivered version: `0.19.0-rc.7`.
Integrated tests and native desktop/phone-preview verification passed.

The new explicit Practice mode starts with one human and fills vacant seats with
labelled server-controlled heroes. Bots navigate the actual arena and use ordinary
hero combat. Late humans replace bots with separate identity and statistics;
reconnect and round turnover retain human ownership. A bounded lifetime roster
causes a safe fresh round when repeated replacements would overflow it.

`make practice` supervises the native server/client session. `make practice-server`
hosts a persistent testing arena. Existing `make play` / `make play-bots` retain
their legacy external-harness behavior. Release does not automatically add the
new server bots. Practice receipts stay local and grant no permanent progress,
rating or database history, even with PostgreSQL configured.

Team and Match chat use server-derived identities, bounded Unicode text,
recipient filtering, request deduplication and a shared chat/reaction token budget.
Signed account social actions are handled locally rather than through the SQL
worker. Scoped subscription opts compatible clients into a separate 4 Hz framed
social stream; old clients receive no unsolicited new packet type. The server
retains 32 events for 30 seconds. Unsigned requests cannot poison a signed
sender's request sequence.

The native UI adds Chat and Reactions buttons, Enter/T keyboard entry, numbered
wheel choices and a touch hold/drag/release wheel over the local hero. UI input
ownership covers open and closing frames. Reaction age and event identity keep
repeated snapshots from renewing bubbles. Rendered layout and scripted input were
checked in nine native captures with real server echoes; a phone-sized native
window is not a physical-device test. BOT labels render behind the HUD.

The versioned shared reaction catalog separates IDs/access policy from local PNG
presentation. Four base reactions are free. The production sender currently
uses empty trusted entitlements, so no protected pack is enabled. Operator and
exact-avatar companion grant APIs are bounded and tested as policy; they are
not connected to a live entitlement source. The reserved generic NFT policy
always denies access. No NFT ownership, marketplace or wallet-linking end-to-end
claim is made.

Adding the explicit bot marker also preserves old human receipt JSON by omitting
`is_bot:false`. Storage rejects bots in rated results and freezes the bot/human
marker with participant identity, protecting earlier durable allocations.

The [runbook and authoring guide](../bot-practice-and-social.md) describes commands,
controls, limits, catalog format and future trusted-provider integration. Exact
test commands/results and labelled native captures are recorded in
`.agent/tasks/BOTS-SOCIAL-2026-09-14/`. No deployment, new production dependency,
physical mobile acceptance or generic NFT service is included in this change.

Verification: 599 tests passed in the full workspace regression; final changed-package
checks passed 546 tests, followed by 317 client tests after QA-only refinements.
All 14 PostgreSQL tests and 11 launcher tests passed. Strict Clippy, formatting
and diff checks passed. See the evidence inventory for exact source/capture hashes.
