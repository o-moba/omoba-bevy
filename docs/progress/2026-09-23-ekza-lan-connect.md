# Ekza authorization and Studio LAN integration — 2026-09-23

Version 0.22.0-rc.5. Reproduced structural UI churn: connection/status changes rebuilt the collection grid, including the active button. Removed status from roster identity and update account/wallet labels in place. Added native UIKit Safari handoff and retry for pending approval. Wallet status is now visible.

Both the SDK and Registry account-link service rejected non-loopback HTTP origins. Added explicit exact-private-host local development configuration without changing production settings or adding dependencies. SDK pin: 247ddbf6dbfc9bac95b52b74ce3a0a1321466a24 on the public codex/omoba-lan-connect branch. Xcode Debug reads ignored local settings; Archive ignores them.

The existing local Supabase test accounts and storage were reused. A new Robert · OMOBA LAN VRM was uploaded, processed, published, built for Omoba, approved by the existing game-owner account and returned through account pairing/library. The real SDK downloaded and validated the rendition. A real UDP practice join retained the approved slug in a running ten-player match. Physical iPad browser input remains a user hardware check. No purchase, wallet signature, production migration or deployment was performed.

See docs/ekza-lan.md and .agent/tasks/TASK-EKZA-LAN-CONNECT-2026-09-23 for commands and proof artifacts.
