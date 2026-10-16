# DEV Account API host service — frozen 2026-10-16

Prepare the isolated Account API needed for the integrated owner DEV playtest.

- AC1: The host helper refuses the wrong VPS, production markers, exposed database ports and a privileged/inheriting portal role before mutation.
- AC2: Credentials remain private, stable across retries and absent from arguments/output. The service binds only to loopback; the exact DEV Caddy host exposes signed native endpoints, while website routes require a private ingress key which is stripped before forwarding.
- AC3: Activation requires a completed backup, exact DEV release revision, matching Account API SHA-256 and portal migration 6. Invalid activation must not mutate host state.
- AC4: Focused tests pass, a fresh reviewer verifies the current code, and the runbook documents deployment, credential handling and rollback.

No runtime binary, dependency, production deployment or billing-provider changes. The already frozen binary candidate remains 07aa4082b03924a68d3cfe44da2229feda9561e9; this change only records its host setup.
