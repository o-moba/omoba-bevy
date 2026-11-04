# DEV account wiring — 2026-10-16

Native DEV packages previously scrubbed account-service settings without adding DEV replacements. Desktop, Android and iOS now compile `https://account.dev.omoba.io` and `https://app.dev.omoba.io`. Packaged DEV rejects mixed or production runtime overrides before creating its HTTP transport. Production compilation and the production iOS source guard remain unchanged.

The Account API routes the exact DEV portal to DEV Registry and Studio. Other origins in DEV DNS zones fail closed instead of using production. Explicit loopback fixtures remain loopback-only. Game deployment keeps an existing portal role login; new roles still start NOLOGIN and the migration owner is locked after migration.

Focused evidence is in `.agent/tasks/DEV-ACCOUNT-WIRING-20261016/`. Build source, package checksums, deployment and TestFlight processing are recorded separately by the playtest rollout; this source change alone does not make endpoints or payment providers available.
