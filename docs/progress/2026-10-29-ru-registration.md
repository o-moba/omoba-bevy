# RU registration and native sign-in

Scope: prepare the `dev` candidate's registration journey for the Russian launch.
Website visual redesign, production activation and release publication are separate.
Canonical version stays at the unpublished 0.45.0 candidate, protocol 12. No tag,
installer, mobile build number or production host was changed.

Website-first VK ID, OK and Mail.ru login now creates one fresh default-stat profile
for a new verified subject. Repeated and concurrent sign-in retains that profile.
The security-definer registration function is bounded; the portal runtime receives
EXECUTE without general profile creation or stat-update privileges. Portal schema
10 is required. The release must apply the matching migration and runtime grants.

Game-first sign-in opens the system browser directly and preserves existing progress
through signed confirmation. The portal reviews the device fingerprint and account;
cross-tab account changes require a new review. Old browser identities are replaced,
revoked keys cannot win a linking race, and native temporary failures retry the same
unexpired enrollment. Status polls have a separate quota from completion writes.
Provider tokens stay server-side; native sessions use their own device keys.

Verification uses a real RU production portal build, Account API, PostgreSQL 18 and
Chrome with three independent loopback provider subjects. This exercises application
integration, not actual VK/OK/Mail.ru accounts. The signed game harness uses exported
private synthetic keys with the real lobby and allocated workers. Raw proof and the
frozen acceptance criteria are in `.agent/tasks/RU-REGISTRATION-20261028/`.

The full game gate passed: 2,388 Rust tests, Python script suite 612 tests (9 skipped)
and 55 iOS tooling tests. The full disposable PostgreSQL gate passed 517 tests with
none skipped; the RU API also passed 38 unit, 8 device and 10 social tests. The final
harness has three separate guards and Clippy checks.
The fresh signed journey passed for VK, OK, Mail.ru and game-first linking: profile
admission, allocated worker, mandatory draft readiness, running match and movement.
VK also reauthenticated on a fresh socket after the endpoint timeout and retained
the same profile, player and match. All owned lobby/worker processes were stopped.
Portal typecheck, production build and unit suite passed (197 passed, 1 skipped).
Changed auth states were reviewed at one Russian 852×393 viewport. No physical
device, distributed package or real provider is certified by these checks.

Read-only production evidence found `omoba.ru/login` returning 404, missing account
and lobby DNS, and only the old game service active on the new RU VPS. Public RU
registration is not ready for acquisition. Remaining activation gates are DNS/TLS,
portal/API rollout with schema 10 and grants, matching protocol 12 server and client,
VK app callback/configuration, real VK/OK/Mail.ru sign-in, signed downloads and
physical-device browser return/restart. The historical dev activation helper's
schema-version pin also needs a separate approved deployment-tooling update.

See [registration](../registration.md) for the flow and [RU launch checklist](../plans/ru-release-2026-10-28.md)
for the rollout gates. Merging these changes into `dev` is not a deployment.
