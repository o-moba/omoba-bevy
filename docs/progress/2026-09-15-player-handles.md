# Native input stability and public player handles — 2026-09-15

## Implemented

- Native friend/profile fields update text in place instead of recreating the
  modal per keystroke. Preserve focus, IME ownership and scroll; bounded Unicode
  input and fixed-height fields apply to desktop and mobile interface variants.
- Editable `nickname#1234` in game profiles and web settings, randomly assigned
  initial adjective/animal names and four-digit tags. Exact authenticated lookup
  precedes friend invitation, whose recipient is the stable internal profile ID.
- Core migration 002 assigns handles to legacy profiles without replacing account
  identities, ratings, relationships or historical match names. ICU uniqueness
  handles Cyrillic case; conflicts preserve the previous value.
- Updated API/OpenAPI, native QA fixtures and portal Russian/English labels.

## Verification

- Native career/identity tests: 33 passed, including per-character entity and
  scroll stability in both interface families, Unicode bounds and stale lookup.
- Shared tests: 56 passed. Account API PostgreSQL tests: 6 passed, including a
  fresh v1-to-v2 migration in a rolled-back fixture schema and concurrent name
  conflicts. Career-store PostgreSQL tests: 11 passed.
- Actual signed UDP test: passed, including full match settlement/history and
  case-insensitive handle lookup. Initial test retry respected the existing
  100 ms account-action throttle; production rate limits were unchanged.
- Web security tests: 3 passed; TypeScript and production build passed.
  Real Chrome/BFF/API/PostgreSQL flow verified full Unicode rename, stable ID and
  rating, readable share text, exact lookup and conflict rollback at 390px.
- API Clippy with warnings denied passed. Native desktop build passed.
- Native QA captures cover six views in desktop 1280×720 and mobile-preview
  844×390 layouts; profile and friend names use full tags. These are marked
  synthetic fixtures, not real-match screenshots or physical-device tests.

## Evidence and rollout

Raw logs and native captures: `.agent/tasks/PLAYER-HANDLES-2026-09-15/`.
Browser checks and screenshots: `../omoba-web/.agent/browser/handles/`.
The ignored local test DB had an unreleased intermediate v2 collation definition;
a local-only transactional function/index repair updated it, then the final 002
SQL was tested independently from a fresh v1 fixture. No production DB changed.

Changes are in `omoba-bevy-player-portal` and `omoba-web`, uncommitted. Main and
mobile clones were not merged or deployed. Runtime rollout requires owner core
migration v2, PostgreSQL ICU and matching client/server/API builds. Physical iOS
and Android keyboard behavior still needs device testing.
