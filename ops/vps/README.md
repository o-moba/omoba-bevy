# Small-host OMOBA beta

Target: `ssh vds-eternal`, 77.246.105.57, Linux x86_64. Native clients enter UDP4000; the lobby assigns UDP41000–41001. This is the OMOBA lobby, separate from Ekza Space TCP3001 and Studio HTTPS. Initial cap: two matches, ten slots per match; solo bot play also consumes a room. This cap is an operating limit, not a load SLA.

Build **only** Linux `server` and `omoba-account-api` with pinned Rust1.95.0 and `--release --locked`. The latter runs the existing career+portal schema migrations, not a public HTTP service. Server has no renderer and does not need avatar model files on disk: shipped manifests are embedded, approved hosted assets are checked through the Registry.

Deployment layout:
- `/opt/omoba/releases/<source>/`: immutable binaries and source identity.
- `/opt/omoba/current`: symlink to active release.
- `/opt/omoba/database.env`: root-only runtime connection; systemd reads it before dropping to user `omoba`. Never commit or print it.
- `/var/lib/omoba/matches` and `lobby-outbox`: persistent owner-only manifests/receipts/outboxes.
- PostgreSQL database `omoba` in the existing PostgreSQL16 instance, accessed through localhost2003. `omoba_owner` owns schema, `omoba_game` is restricted runtime, `omoba_portal` is a reserved NOLOGIN role. Existing databases and credentials are preserved.

Before starting, run `omoba-account-api migrate` using a temporary owner credential, apply `account-api/ops/grants.sql` with `game_role=omoba_game` and `portal_role=omoba_portal`, revoke PUBLIC access/CREATE and disable owner LOGIN. Keep migrations separate from normal runtime. Install this unit, `systemctl daemon-reload`, then `systemctl enable --now omoba-lobby`. The entire lobby/worker group is capped at700MiB and150% CPU; normal service restarts stop the workers and use durable interrupted-match recovery. Existing application-level recovery can adopt surviving workers after a lobby-only crash, but this unit intentionally treats a service restart as a group restart.

Checks: `systemctl status omoba-lobby`, `journalctl -u omoba-lobby`, `systemctl show omoba-lobby -p MemoryCurrent -p CPUUsageNSec`, and a real authenticated remote client allocation. An open UDP socket alone does not prove gameplay. Watch simulation/snapshot delays and Studio availability while testing concurrent rooms. Only open UDP4000/41000–41001 if the host/provider firewall requires it; preserve unrelated rules.

Rollback: stop `omoba-lobby`, point `current` at the previous compatible release, start again. Never delete live match directories or drop the database. First deployment has no prior OMOBA service: rollback means stopping/disabling this service and leaving durable state for inspection. Studio/API are unaffected. Back up the dedicated database and match/outbox state before schema changes; current deployment creates a fresh database only.

Client: fresh installations use77.246.105.57:4000. Home's server button opens a shared editor with **OMOBA Beta** and **Localhost** presets; presets fill the editable field, **Connect** applies and saves it. Desktop party lobby also retains its address editor. Priority: non-empty runtime `GAME_SERVER_ADDR`, valid saved preference, build override `OMOBA_DEFAULT_GAME_SERVER_ADDR`, built-in beta address. Existing local/custom preferences are preserved. Choose Beta once to move an older install; the next launch remembers it. Temporary worker endpoints never overwrite the saved lobby.
