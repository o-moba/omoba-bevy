# OMOBA launch readiness

Status: specification, 2026-10-24. Source: launch-readiness audit of `feat/market-profiles` @3bd0958 (game), `market-web` (portal), `landing-v2` (landing) and `wotori/community` (community hub). The audit covered matchmaking, ranked, bots, server ops, client, community, security and RU legal.

**Goal.** Players open the game and find a match: Ranked, Casual PvP or vs Bots, solo or in a party. History is saved, rating grows, and feedback reaches the owner. The product runs on its own without anyone watching it. RU is the first market.

**How to read this document**
- Paths are relative to the game repo. `market-web/…` is the portal repo, `landing-v2/…` the landing repo and `community/…` the community hub.
- Severity: **blocker** means no public announcement before it is done; **major** means it must be fixed for a product that runs unattended; **minor** is polish or growth work.
- Size: **S** is about 0.5 engineer-weeks, **M** about 1.5, **L** about 3-4.
- Owner: **code** (engineers), **ops** (host, deploy, monitoring) or **owner** (decisions, legal, accounts).
- When several audit findings described the same gap, they were merged into one spec. A merged spec keeps the strongest severity and all the evidence.

---

## 1. Verdict

OMOBA cannot launch as a self-running product today. The engine is close, but the product around it is not ready. The RU servers do not exist. The RU portal cannot reach the Account API, so VK sign-in, device approval and recovery codes all fail. Nothing backs up the career database. Russian law requires a privacy policy, a user agreement and a notice to Roskomnadzor, and none of these exist yet. The game core is solid and should be kept: exactly-once settlement, crash recovery, server-authoritative anti-cheat and the lobby/worker split. What the product lacks:
- Players cannot choose Ranked or Casual. Rated games happen only by accident, when 10 humans happen to queue together.
- The RU host runs 2 arenas, and every solo bot game takes a whole arena.
- Clients have no way to update when the server changes.
- The owner has no moderation tools, no monitoring and no feedback channel.

**Critical path.** Two tracks start at the same time:
- **Owner/legal:** LR-05 → LR-04 → LR-06/LR-19. Its calendar time does not depend on engineering, so it should start first.
- **Infra:** LR-01 → LR-02/LR-03/LR-17 → LR-09.

The remaining M0 work runs in parallel with these: queues (LR-08), match lifecycle (LR-11/12/13), update path (LR-14), moderation and abuse limits (LR-15/16), and the client/web entry (LR-18/20/22/24/25). M0 is about 24 engineer-weeks, roughly 9-10 calendar weeks with three engineers.

---

## 2. Already works (keep, do not rebuild)

- **Lobby/worker architecture.** A single lobby runs separate worker processes, each with an immutable manifest and atomic status receipts. It also has a coordinator lock, worker adoption and stale-heartbeat recovery with a bind probe (`server/src/match_pool.rs`, `server/src/match_allocation.rs`).
- **Exactly-once settlement.** Intent is staged before settlement, profiles are locked in sorted order, results are written in one transaction, and triggers make the match history append-only (`career-store/src/career_store.rs:527-702,1010-1032`, `career-store/migrations/postgres/001_career.sql:73-106`).
- **Durable outbox and recovery.** Match receipts persist through a worker crash, a lobby crash or a database outage. A crashed match settles as Interrupted and unrated, never lost and never counted twice (`server/src/career_backend.rs:1167-1260,1380-1441`).
- **Fail-closed rated eligibility.** A match is rated only with a full, authenticated, bot-free roster on the default map without dev modifiers. The store checks the newcomer cohort and the rating spread again (`server/src/career_runtime.rs:523-564`, `career-store/src/career_store.rs:1207-1231`).
- **Bounded team Elo (K=32, 0-5000) and team balancing.** Parties are never split across teams (`career-store/src/matchmaking.rs`, `server/src/match_service.rs:422-479`).
- **Progress at zero population.** Quick fills with bots after 30 s. Bot games award XP and save history under public-casual-v1 but never change rating (`server/src/match_service.rs:14,481-529`).
- **Disconnects.** A bot takes over a disconnected seat. The player can reclaim it within 180 s, and Resume works after a client restart (`server/src/session_recovery.rs`, `client/src/net/recovery.rs`).
- **Prematch timeouts.** The draft has 30 s timeouts and a 180 s join deadline, so prematch cannot stall forever (`server/src/prematch.rs`).
- **Shared bot brain.** Online, standalone and offline play all use one bot brain. Four difficulty levels are calibrated, and offline practice with a sandbox needs no server (`common/src/bots.rs`, `shared/src/bot_profile.rs`, `common/src/offline.rs`).
- **Compatibility preflight.** Every transport checks compatibility first and reports the exact mismatch category (`shared/src/compatibility.rs`, `docs/release-compatibility.md`).
- **Server-authoritative anti-cheat.**
  - Commands are Ed25519-signed and bound to session and sequence.
  - The server enforces movement envelopes, cast validation and server-side fog of war.
  - Debug commands are refused in worker rounds.
  - Evidence: `shared/src/public_transport.rs`, `common/src/session.rs:289-360`, `common/src/vision.rs`.
- **UDP admission hardening.** The return path is validated, replies are never amplified, and each endpoint is limited to 120 packets per second (`server/src/public_transport.rs`).
- **Account security.**
  - Device enrollment is signed, and sensitive actions need a fresh session.
  - Recovery codes are stored as HMACs.
  - VK ID uses PKCE, stores only an HMAC of the subject, and needs a game-signed confirmation to attach.
  - Evidence: `account-api/src/devices.rs`, `account-api/src/social.rs`.
- **Account API and portal BFF.** The Account API binds to loopback only, uses timeouts, logs no request paths, has a database-backed rate limiter and writes audit events. The portal BFF uses `__Host-` cookies, CSRF protection and a path allowlist.
- **Social features.** Server-authoritative chat with limits, local mute, server-owned parties, durable friends, `Nickname#1234` handles, and privacy-by-default profiles.
- **Player portal.** Match history, full match reports and statistics, built from an idempotent projection.
- **Market gating and localization.**
  - Market gating fails closed: the RU build has no Ekza, no payments and VK ID enabled.
  - Russian localization is complete, with en/ru/zh at 1262 keys each and strict i18n checks.
- **Release pipeline.** One command builds per market and produces SHA-256 sums, a compatibility manifest and a verified download catalog. The dev-contour deployer `ops/vps/dev/lobby_dev.py` is the template for production.
- **Systemd units.** They are sandboxed with memory, CPU and task caps.
- **Measured baselines.** On a dev Mac (M4 Pro), 10 all-human arenas ran at a 65 ms p95 snapshot gap (`docs/public-mvp.md:111-125`).

---

## 3. Specs

### M0 — Launch blockers (before any public announcement)

#### LR-01 — RU production stack provisioned and self-starting
Area: hosting/deployment · **blocker** · M · ops (owner buys the VPS and sets DNS)
- **Now:**
  - The RU hostnames are baked into builds (`lobby.omoba.ru:4000`, `account.omoba.ru`, `omoba.ru`), but nothing is deployed (`docs/markets.md:16-17,23-25`; `docs/progress/2026-10-24-market-profiles.md:53`).
  - RU builds refuse the global lobby (`docs/markets.md:39-40`), so an RU client has nowhere to connect.
  - `ops/vps/ru` has units only for the lobby and the Account API. The portal is "run as its own service" with no unit (`ops/vps/ru/README.md:34-36`), and `market-web/scripts/start.mjs` exits when its child exits.
  - Units wait only for `network-online.target`, not for `postgresql.service`. The lobby has `StartLimitBurst=6` in 120 s.
  - There is no host bootstrap: Postgres ICU (`docs/player-handles.md:43-44`), Caddy, firewall and SSH are all manual.
  - `OMOBA_MATCH_PUBLIC_HOST` defaults to `127.0.0.1` without a loopback check (`server/src/match_pool.rs:49-59`).
  - The release gate `docs/release-gate-checklist.md` dates from April and is stale. The only current check is runbook step 6 (`docs/markets.md:139-141`).
- **Requirement:** Postgres, the lobby with its workers, the Account API and the portal run on a host in Russia. Each service restarts on its own after a crash or a reboot, and RU packages connect to it.
- **Acceptance:**
  - [ ] `make release-ru-preflight` prints `ready:true`, and `compatibility -- check lobby.omoba.ru:4000 …` exits 0 against the lobby and one worker. Needs the host: release checklist rows S6–S8.
  - [x] `ops/vps/ru/omoba-portal-ru.service` exists, with `Restart=always`, memory caps and a dedicated user.
  - [x] The lobby and Account API units have `After=`/`Wants=postgresql.service`.
  - [ ] A reboot test brings everything up with no manual step. The test is `ops/vps/ru/ru_host.py reboot-check --apply` (simulated in `scripts/test_ru_host.py`); the real run needs the host (checklist row S9).
  - [x] A bootstrap script or section installs and verifies (`ops/vps/ru/ru_host.py bootstrap`, proven against a fake Ubuntu host and its SQL against real PostgreSQL; the first real run is on the RU host):
    - PostgreSQL 16+ with `und-x-icu`
    - Caddy
    - firewall open only for UDP 4000, the arena range and TCP 443 (plus SSH)
    - key-only SSH
  - [x] A public lobby with an unset or loopback public host and a non-loopback bind exits with a clear error (unit test).
  - [x] A written RU release checklist (`ops/vps/ru/release-checklist.md`) replaces the April gate. For each platform it covers:
    - clean install and first run in Russian
    - VK sign-in
    - a bot match and the casual and ranked queues
    - reconnect
    - an update over the previous version
    - the incompatible-server prompt

    The result is stored in `docs/progress` for every release.
  - [ ] An RU client plays a server bot match and a 2-client casual match, and the history shows on `omoba.ru`. Needs the host: checklist rows P3 and P4.
- **Depends on:** — (owner checklist: VPS and DNS)

#### LR-02 — Portal reaches the Account API with its key and the real client IP
Area: portal/accounts · **blocker** · S · code+ops
- **Now:**
  - `accountIngressHeaders()` sends `X-Omoba-Portal-Key` only in the dev contour (`market-web/src/lib/security.mjs:51-55`; the test `market-web/tests/account-ingress.test.mjs:21-24` asserts this).
  - `Caddyfile.ru` requires that key on non-native `/v1/*` routes (`ops/vps/ru/Caddyfile.ru:13-16,27-38`). As a result, every RU portal call returns 403: VK callback, `/connect`, recovery codes and `/me`.
  - Once that is fixed, Caddy still overwrites `X-Real-IP` with the portal's own address (`Caddyfile.ru:27-31`; `account-api/src/lib.rs:351-360`). The per-IP buckets for social sign-in (30/min) and pairing creation (5/min) then become market-wide (`account-api/src/lib.rs:382-391,410-420`).
- **Requirement:** Pick one topology and ship it.
  - **Preferred:** the portal calls `http://127.0.0.1:40550`, `OMOBA_TRUST_LOOPBACK_PROXY=1` is set on both sides, and Caddy on `app.ru` sets `X-Real-IP`.
  - **Alternative:** a production branch in `accountIngressHeaders`, with Caddy keeping a portal-supplied `X-Real-IP` only when the key is valid.

  The portal fails at startup if a required key is missing or malformed.
- **Acceptance:**
  - [ ] A unit test covers the chosen config with `OMOBA_MARKET=ru` in the production contour.
  - [ ] `portalConfig()` throws for a non-loopback API without a valid key.
  - [ ] End-to-end behind `Caddyfile.ru`:
    - `GET /api/me` works
    - `POST /api/me/recovery-codes` returns 8 codes
    - the VK callback reaches `/v1/auth/social/vk`
    - no 403 anywhere
  - [ ] 31 VK callbacks from IP A within one minute do not block IP B.
  - [ ] `release.py preflight --market ru` fails on a 403 from a key-gated route.
  - [ ] The topology is documented in `portal.env.example` and `ops/vps/ru/README.md`.
- **Depends on:** LR-01

#### LR-03 — Automated off-host backups, secrets escrow and restore drill
Area: persistence · **blocker** · S · ops
- **Now:**
  - Postgres shares the RU VPS (`ops/vps/ru/account-api.env.example:3`; `ops/vps/ru/README.md:18-26`).
  - The only backup guidance is "pg_dump -Fc before schema changes" (`ops/vps/ru/README.md:55-56`; `account-api/README.md:134-147`).
  - No backup timer, WAL archiving or off-host copy exists in `ops/`, `scripts/` or the `Makefile`. `docs/public-mvp.md:129-131` lists a restore drill as pre-launch work.
  - `OMOBA_PORTAL_SECRET` and `OMOBA_SOCIAL_SECRET` are not escrowed. The social secret must never be rotated (`ops/vps/ru/README.md:35-36`), so losing it orphans every VK identity.
  - `/var/lib/omoba-ru/matches` is not backed up.
  - The game database role still has broad UPDATE rights (`account-api/ops/grants.sql:30-36`); see LR-63.
- **Requirement:**
  - An encrypted `pg_dump -Fc` runs nightly and before each migration, and goes to storage in Russia.
  - Retention: 14 daily and 8 weekly copies.
  - The match and outbox archive is included.
  - Secrets (including the Android release key from LR-24) are escrowed offline.
  - A restore into a scratch database runs automatically every month.
  - A failed backup raises an alert.
- **Acceptance:**
  - [ ] A systemd timer is shipped in `ops/vps/ru`, and backups for the last N days exist off-host. Shipped: `omoba-backup-ru.timer` (nightly), `omoba-restore-drill-ru.timer` (monthly) and `omoba-backup-check-ru.timer` (hourly), installed by `ru_host.py bootstrap`; `ru_host.py ship` backs up before every migration. Off-host sets need the host and the owner's storage in Russia (`ru_backup.py check --remote` and `ru_host.py status` count them).
  - [x] The restore script checks schema versions and row counts (`ru_backup.py drill`: tables, row counts, a digest of every table's rows, both schema version lists, the state archive; proven against real PostgreSQL in `scripts/test_ru_backup.py`).
  - [ ] A restore onto a scratch host keeps ratings, history counts, an existing recovery code and VK sign-in working. The PostgreSQL round-trip test restores ratings, match participants, a recovery code hash and a VK identity row unchanged; the sign-in checks need the host (README › Restore, onto a new host).
  - [x] An alert fires when no backup has run for more than 26 h: the hourly `ru_backup.py check --alert` (journal, and Telegram once `OMOBA_TELEGRAM_BOT_TOKEN` is set); LR-17 can reuse it.
  - [ ] `RUNBOOK.md` and the ops README name the backup location, the escrow holder and the restore steps. Location and restore steps are written; the escrow holder is the owner's to name (README escrow table).
- **Depends on:** LR-01, LR-17

#### LR-04 — Russian legal documents
Area: legal (152-FZ) · **blocker** · M · owner
- **Now:**
  - The game and the portal have no RU privacy policy, user agreement or consent document (`docs/markets.md:121-125`; `docs/progress/2026-10-24-market-profiles.md:50-53`).
  - `landing-v2/apps/omoba/src/app/privacy/page.tsx:55-66` covers only the website.
  - `market-web/src/app` has no privacy or terms route.
- **Requirement:** The owner and counsel approve three documents in Russian:
  - a personal-data processing policy (152-FZ art. 18.1)
  - a user agreement covering conduct and chat rules, a sanctions ladder, liability and the relation to AGPL/MPL
  - a separate consent to processing; counsel confirms its form

  The documents list all data actually processed:
  - the device public key and `Nickname#tag`
  - match history and stats
  - the VK subject HMAC and the temporary VK display name
  - IP-derived rate-limit hashes and session/locale cookies
  - in-memory chat
  - feedback, crash and analytics data (LR-20/29/30)
  - log retention

  They also name the operator and its address, a contact e-mail, the storage location (RU VPS), retention per data type and any cross-border transfers. Finally, they set out a procedure for data-subject requests: deletion is a manual operator procedure until LR-42 ships.
- **Acceptance:**
  - [ ] Published at stable Russian URLs (e.g. `omoba.ru/privacy`, `/terms`) with a version and an effective date.
  - [ ] Every data category in `account-api/migrations/001-007` and `docs/bot-practice-and-social.md` (chat) is covered.
  - [ ] Counsel review (date and reviewer) is recorded in `docs/markets.md`, RU runbook step 1.
- **Depends on:** LR-05

#### LR-05 — Personal-data operator entity and Roskomnadzor notice
Area: legal (152-FZ) · **blocker** · S · owner
- **Now:**
  - No operator entity exists. The landing credits "Wotori Studio", and the only contact is Telegram (`landing-v2/apps/omoba/src/app/privacy/page.tsx:160-176`).
  - The RKN notice is listed as an owner step, with no record that it was filed (`docs/markets.md:121-125`).
- **Requirement:**
  - Decide the entity: ИП or ООО. Self-employed (НПД) status is not enough once the RU market sells supporter auras: the YooKassa shop with «Чеки от ЮKassa» (LR-67) needs an ИП or ООО. The same entity is used for the YooKassa, RuStore, VK ID and VK Play accounts.
  - File the operator notice (152-FZ art. 22).
  - Either file a cross-border notice or state that no RU personal data leaves Russia. Check off-site backups, Pinata, and Vercel/Supabase analytics on RU-facing pages.
- **Acceptance:**
  - [ ] The registry entry is linked in the owner checklist.
  - [ ] The operator's name, address and support e-mail appear in the policy and in the RU portal footer.
  - [ ] The cross-border position is recorded.
- **Depends on:** —

#### LR-06 — Consent screen and legal links in client, portal and packages
Area: legal consent UX · **blocker** · M · code
- **Now:**
  - The client has no consent, terms or privacy strings. The only "Принять" in `client/i18n/ru` is for friend and party invites.
  - The portal login card has no legal notice (`market-web/src/components/portal.tsx:58-62`), and the footer reads only "OMOBA · Website ↗" (`portal.tsx:224`).
  - The README ends with licenses (`scripts/release.py:583`).
  - Neither `shared/src/market.rs` nor `scripts/markets.py` has policy URL fields.
  - `SOURCE.md` ships in packages (`scripts/package_licenses.py:17`), and the client shows the build revision (`client/src/build_info.rs:54-67`), but there is no visible source link.
- **Requirement:**
  - Add per-market URLs for privacy, terms, source and support in `shared/src/market.rs`, `scripts/markets.py` and `market-web` `market-profile.mjs`.
  - RU client: a first-launch acceptance screen before any lobby or Account API traffic. The accepted version is stored locally and, once online, server-side. A version bump asks again.
  - Portal: a notice with links next to VK login and `/connect`, and Privacy, Terms, Support and Source in the footer.
  - Client Settings → About: privacy, terms, a source link to the public tag `v<version>-ru`, and the build label.
- **Acceptance:**
  - [x] RU build test: a fresh profile cannot queue or link VK before accepting, and the accepted version persists. Game half: the first-launch card (`client/src/legal.rs`); the lobby start, session lifecycle, menu retry, release-catalog read and career actions (VK, devices, pairing) carry `bound_and_accepted` (plugin tests per module); the record `{region, doc_version, accepted_at}` is `legal_acceptance.json` in the region folder. No server-side record yet (follow-up). UI-tree tests only; no visual check, the Mac's screen is locked.
  - [x] A version bump shows the screen again (`version` in the market table; test with a bumped version).
  - [ ] RU portal end-to-end: `/privacy` and `/terms` render in Russian; login and `/connect` show the notice; the footer links work.
  - [ ] The global build is unchanged (market-gated). Game half done: the global table is empty, so no card, no About page, no gate and an unchanged README (tests in both markets); the portal half is LR-06's omoba-web part.
  - [ ] The public tag `v<version>-ru` exists before publication. Tooling done: `release.py catalog --market ru` refuses until the tag is on GitHub (`require_public_source_tag`); creating the tag at release time is the owner's.
- **Depends on:** LR-04 (placeholder versions are enough to build against), LR-02

#### LR-07 — Age category marking (436-FZ)
Area: legal · **major** · S · owner
- **Now:**
  - No age sign appears in the game, the portal, the landing or the packages.
  - The store age rating is left to the owner (`docs/markets.md:143-146`).
  - Unmoderated chat with strangers ships (`docs/bot-practice-and-social.md:97-116`).
- **Requirement:** The owner picks a category with counsel, taking the chat decision (LR-41) into account. The sign appears on:
  - the RU landing and `/download`
  - store listings
  - the RU client boot or first-launch screen
- **Acceptance:**
  - [ ] The category and its rationale are recorded in `docs/markets.md`.
  - [ ] The sign is visible on every surface listed above.
- **Depends on:** O-9 (chat decision); shown through LR-22

#### LR-08 — Player-chosen Ranked, Casual PvP and vs Bots queues
Area: matchmaking/modes · **blocker** · M · code (owner sets the policy)
- **Now:**
  - The wire format already has `MatchPreference` Quick, HumansOnly and BotPractice (`shared/src/match_service.rs:5-12`).
  - Home and the party lobby send only Quick or BotPractice (`client/src/frontend/home.rs:1180-1192,1215-1225`; `client/src/frontend/lobby.rs:544-562`).
  - Commit 52bf02d removed the HumansOnly button, leaving the `home.button.humans_only` key orphaned in en/ru/zh.
  - Head count decides whether a match is rated:
    - the allocation is rated when `humans.len()==10` (`server/src/match_service.rs:227`)
    - the worker switches to release mode on the same condition (`server/src/match_pool.rs:334-341`)
    - eligibility ignores the chosen preference (`server/src/career_runtime.rs:522-562`)
  - Quick bot-fills after 30 s (`match_service.rs:14,510-520`), so rated games almost never form, and nobody can choose an unrated 10-human game.
  - Quick also applies the 300-point and newcomer cohort filter (`match_service.rs:372-376`), which splits the few humans online into separate bot games.
  - A party {newcomer 1000, veteran 1400} plus 8 solo players produces a rated roster that the store rejects. Every player is then kicked with the English message "Queued ratings changed…" (`match_service.rs:380-405,481-500`; `career-store/src/career_store.rs:1059-1080`; `server/src/career_runtime.rs:1093-1106`).
  - `MatchAllocation.rated` reaches the client but is shown only on the result screen (`client/src/career.rs:1557`).
  - `docs/features.md:390` and `docs/public-mvp.md:11` advertise the removed "Wait for players" mode.
- **Requirement:**
  - Home and the party lobby offer three queues:
    - **Ranked:** humans only, rated, never bot-filled
    - **Casual PvP:** unrated, bot fill allowed, awards XP
    - **vs Bots**
  - `rated == (preference == Ranked) && eligibility`.
  - Casual groups every human in the queue, whatever their rating or newcomer cohort, before filling with bots.
  - The lobby and the store use one cohort policy: the career-store matchmaking constants, with no literal 300 in `server/src/match_service.rs`. Mixed parties never produce a rated manifest the store will reject.
  - Searching, draft and loading show the queue and a Rated/Unrated badge.
  - The owner's low-population ranked policy (O-1) is recorded in `docs/match-progression.md`.
- **Acceptance:**
  - [x] Distinct Ranked and Casual variants exist. Old clients' Quick/HumansOnly either keeps working on the wire or receives update-required (LR-14). Done: `MatchPreference::{Casual, Ranked}` keep the wire names `quick`/`humans_only`; every new field is additive, so protocol 11 stays.
  - [x] Unit tests:
    - 10 humans in Casual → unrated
    - 10 humans in Ranked → rated
    - Ranked never bot-fills
  - [x] One newcomer and one veteran in Casual after 30 s get one allocation with 2 humans.
  - [x] A party {1000 newcomer, 1400 veteran} plus 8 solo players never yields a rated manifest the store rejects.
  - [ ] Frontend QA captures in en and ru show the three choices and the rated badge. UI-tree tests cover both languages; the native captures need an unlocked screen.
  - [x] `docs/features.md` and `docs/public-mvp.md` describe the shipped modes, and the orphan key is either used or removed.
- **Depends on:** O-1, O-2

#### LR-09 — Arena capacity measured and sized on the RU host
Area: capacity · **blocker** · M · ops (+S code)
- **Now:**
  - `OMOBA_MATCH_CAPACITY=2` is set in `ops/vps/ru/omoba-lobby-ru.service:27-28` and `ops/vps/omoba-lobby.service:24`.
  - The lobby and all workers share `MemoryMax=700M` and `CPUQuota=150%` (`:35-37`).
  - Only UDP 41000-41001 is opened (`ops/vps/ru/README.md:41`).
  - The code default is 16 (`server/src/match_pool.rs:41`). `docs/public-mvp.md:111-125` says to keep it until the real host is measured, and the only measurement so far is on a dev M4 Pro.
  - The host is 2 vCPU / 4 GB, shared with Postgres, the Account API and the portal (`docs/markets.md:123`).
  - Each solo bot practice or bot-filled Quick takes a whole arena (`server/src/match_service.rs:494-502,508-517`).
  - A slot keeps its port for up to 35 s after the match ends (`match_pool.rs:113-115,267-273`).
  - When the pool is full, the player sees "All arenas busy" with no position, no ETA and no offline option (`client/src/match_service.rs:328-331`).
  - The endpoint ceiling (`server/src/public_transport.rs:15,131-133`) and the career-client ceiling (`server/src/career_backend.rs:26,649-651`) drop players silently.
- **Requirement:**
  - Measure bot-heavy and mixed load on the RU host or an identical one.
  - Set capacity, the port range and the cgroup limits from that measurement, and document them.
  - With 20 concurrent players in any mix of modes, nobody waits more than 60 s for an arena.
  - A full pool shows a queue position or ETA and offers Offline practice in one tap.
  - Both ceilings reply "server full" instead of dropping.
- **Acceptance:**
  - [ ] `scripts/test_public_mvp.mjs --preference=bot_practice` and a mixed run on the RU host record, per arena type:
    - snapshot-gap p95/p99
    - CPU and RSS
    - snapshot bytes per second per player (input to LR-28)

    p95 stays under 100 ms at the chosen capacity.
  - [ ] `ops/vps/ru/README.md` states the capacity, the UDP range, MemoryMax/CPUQuota and the measured per-arena cost.
  - [ ] 20 simulated solo Bot-practice players all reach Running within 60 s.
  - [ ] With the pool full, the searching screen shows a position or ETA and "Play offline practice while you wait".
  - [ ] At the endpoint or career-client ceiling, the lobby replies "server full" (test).
  - [ ] A capacity-busy alert fires through LR-17.
- **Depends on:** LR-01, LR-10, LR-11 (re-measure after both), LR-17

#### LR-10 — Bot route-planning CPU budget
Area: bot performance · **major** · M · code
- **Now:**
  - A bot replans whenever its route is empty (`common/src/bots.rs:2156-2165`), even when the last plan failed or the goal was already reached.
  - Every A* plan allocates full-grid buffers (`shared/src/navigation/search.rs:141-175`).
  - Measured 166-299 plans per bot-minute, which accounts for most of the CPU time (`docs/progress/2026-10-14-bot-personas.md:91`).
  - A solo arena runs 9 bots.
- **Requirement:**
  - At most one plan per `ROUTE_INTERVAL` per bot, including after a failure.
  - Reuse buffers: the static grid plus an overlay of dynamic discs.
- **Acceptance:**
  - [x] The simulator shows ≤ 80 plans per bot-minute at every level.
  - [x] The hot path does not allocate, or uses per-thread buffers.
  - [x] Elite trace-equivalence and parity tests stay green, or the baseline is re-recorded with a justification.
  - [ ] The LR-09 probe shows more arenas per vCPU.
  - [ ] Offline frame time is measured on a low-end Android device.
- **Depends on:** —

#### LR-11 — Every match ends: abandoned-match reaping and a hard duration cap
Area: worker lifecycle · **major** · S · code
- **Now:**
  - The empty-roster timer starts only when no joined non-bot player remains. A human seat taken over by a bot, or one the human left, keeps `is_bot=false, joined=true` (`server/src/session.rs:69-99`; `server/src/match_allocation.rs:574-597,749-753`). So a match all humans have left keeps simulating bot against bot.
  - Victory happens only when a BaseTower falls (`common/src/sim/towers.rs:75-79`).
  - There is no cap on match length or worker lifetime.
  - In the simulator, 1 of 200 Elite-vs-Elite rounds reached Victory within 20 min (`docs/progress/2026-10-14-bot-personas.md:89`).
- **Requirement:**
  - A worker with no connected human for `SESSION_RECLAIM_WINDOW` plus a grace period settles as abandoned (unrated) and frees its port.
  - A configurable hard maximum duration ends any match with a defined outcome.
- **Acceptance:**
  - [x] Test: in a BotPractice match the human leaves; after the reclaim window and grace, the status is Finished or Failed and the port is released.
  - [x] Test: a match past the max duration reaches a terminal state, with no rating for an abandoned bot game.
  - [x] `MATCH_METRIC` emits `abandoned_reaped` and `duration_capped`.
- **Depends on:** —

#### LR-12 — Watchdog and self-healing processes
Area: crash recovery · **major** · S · code
- **Now:**
  - The lobby is a single loop under `Type=simple`, with no `WatchdogSec` or `sd_notify` (`ops/vps/ru/omoba-lobby-ru.service:15,31-33`).
  - A worker with a stale heartbeat is relaunched only when the lobby holds no child for it. A hung child is never killed and keeps its port (`server/src/match_pool.rs:209-221,251-257`).
  - `server/src` has no panic hook or `catch_unwind`.
  - With 2 arenas, one hung worker halves capacity.
- **Requirement:**
  - The lobby sends watchdog pings.
  - A worker with a stale heartbeat is killed and recovered as interrupted.
  - A panic hook writes a crash file and raises an alert.
- **Acceptance:**
  - [ ] With `Type=notify` and `WatchdogSec=30`, systemd restarts a stalled loop.
  - [ ] A worker whose heartbeat is older than 120 s while its child is alive is killed and settled as interrupted (harness test).
  - [ ] A worker past its max lifetime ends as interrupted (shares the cap with LR-11).
  - [ ] A panic writes `/var/lib/omoba-ru/crashes/*` and alerts through LR-17.
- **Depends on:** LR-11, LR-17

#### LR-13 — Honest state after leaving a running match
Area: matchmaking/re-queue · **major** · S · code
- **Now:**
  - After a deliberate Leave, a bot keeps playing the hero (`server/src/match_allocation.rs:574-597`).
  - The lobby keeps the assignment, `enqueue` returns silently, and the view stays Assigned (`server/src/match_service.rs:146-155,210-233`).
  - The client skips that allocation and sends FindMatch every 2 s while showing "Match found · connecting…" indefinitely (`client/src/match_service.rs:119-131,234-262,361`).
  - Cancel is refused for a running match (`server/src/match_pool.rs:171-182`).
- **Requirement:** Pressing Play after leaving a running match either shows "You are still in a match: Resume / ends in about N min" or creates a new queue entry. The player never sees a spinner that cannot connect.
- **Acceptance:**
  - [x] A new view state (e.g. `MatchServiceView::InMatch { allocation_id }`) or an error code exists. Done as `MatchServiceView::Assigned { in_match: Some(InMatch { ends_within_secs }) }`, additive so protocol 11 stays.
  - [x] Client test: the case renders a localized message with Resume.
  - [x] Server test: the state is visible within one tick. With LR-11 in place, the seat frees after the reclaim window.
- **Depends on:** LR-11

#### LR-14 — Client update path
Area: install/update · **blocker** · M · code
- **Now:**
  - The client never checks for a newer version.
  - A contract mismatch shows only "Update the game or choose another server" (`client/src/net/status_ui.rs:288-298`; `client/i18n/en/net.json:14-19`), yet production builds have no server selection (`scripts/release.py:22-23`; `RUNBOOK.md:259`).
  - Menus stop retrying after a mismatch (`client/src/frontend/mod.rs:468-485`; `client/src/net/session.rs:746-748`).
  - `GAMEPLAY_REVISION` changed 6 times between 2026-10-05 and 2026-10-11 (`shared/src/compatibility.rs`).
  - `error.join.map_geometry_mismatch` tells players to "Install the matching map release".
  - The release catalog, portal `/download` and `platform::open_external_url` (`client/src/platform.rs:194`) all exist but are not used for updates. There is no `min_release` or `update_url` anywhere.
- **Requirement:**
  - At boot and on any mismatch, the client fetches the market release catalog.
  - It shows a dismissible "Update available" banner, or a blocking "Update required" panel with a button to `<portal>/download` for its platform (or to the store).
  - The compatibility reply carries the minimum release and the update URL.
  - `docs/RELEASING.md` defines which changes bump `GAMEPLAY_REVISION`, says clients ship before the server, and requires a player notice before a forced update. The alternative is to move balance values into server data (O-12).
- **Acceptance:**
  - [x] On a mismatch, the RU client shows "Требуется обновление" with a button to `omoba.ru/download` for its platform. Done as Home's update panel: the download button replaces Play and opens `<portal>/download?platform=…` on the portal compiled from `scripts/markets.py` (UI-tree test in Russian; no visual check, the Mac's screen is locked).
  - [x] A newer compatible build shows a banner on Home. Dismissible with **Later**; read from `GET /v1/releases` at boot and after a mismatch.
  - [x] The compatibility reply, or a signed document, carries min release and update URL, with tests for bad and expired documents. Done in the reply (`UpdateNotice`, additive on handshake 1). Tests cover bad notices and catalogs, stale-nonce reports and stale catalogs; no signed document was added.
  - [x] No production string mentions choosing a server, and all strings exist in en, ru and zh-Hans.
  - [x] The RELEASING.md rule is recorded ("Updates and contract changes").
  - [x] LR-68 rules (section 7). The catalog URL is the build's `account_api`. The reply stays `ReleaseContract` with the additive `UpdateNotice` and no protocol bump. Catalog entries carry `channel` (`direct`), `min_release` and `update_url` (`release.py catalog --min-release`; validated by the Account API and `passport::release_catalog`, absent fields tolerated). The client reads only its own channel's entries and picks the button target per (channel, platform): a direct build stays on its portal, and a store build would take only its catalog page, never a web download. A build with no link names the App Store or TestFlight only on iPhone; elsewhere it names the official site. `region` on entries stays with LR-23.
- **Depends on:** LR-01, LR-23

#### LR-15 — Moderation minimum: sanctions, chat kill switch, operator CLI
Area: moderation/admin · **blocker** · M · code (+ops runbook)
- **Now:**
  - No sanction exists at profile level. `career_profiles` has no status or ban column (`career-store/migrations/postgres/001_career.sql:2-12`).
  - The only lever is `career_keys.revoked_at` (`003_device_keys.sql:2`), which a reinstall evades because an unknown key creates a new profile (`career-store/src/career_store.rs:296-319`).
  - The Account API has no admin route and only a `migrate` CLI (`account-api/src/lib.rs:439-551`; `account-api/src/main.rs:13`).
  - The chat path checks no sanction (`server/src/social.rs:136-192,234-265`), and there is no chat kill switch.
  - `RUNBOOK.md` has no moderation section.
  - Audit events are pruned after 30 days (`account-api/src/lib.rs:217`).
- **Requirement:**
  - Add a `sanctions` table: profile_id, kind (`chat_mute|matchmaking_ban|ranked_ban|account_ban|name_reset`), reason, expires_at, created_by. Each sanction writes an audit row.
  - Enforce sanctions in four places:
    - lobby admission and FindMatch, with a localized reason and expiry
    - the chat sender
    - Account API authentication (account ban)
    - party presence (sanctioned players hidden)
  - A server-wide chat kill switch, ideally with no restart needed.
  - `omoba-account-api` CLI commands: `sanction add/list/lift` and force-rename to `Player#NNNN`.
  - Sanction audit rows are kept for at least a year.
  - A RUNBOOK section covers looking up a player, applying or lifting a sanction, and turning chat off.
- **Acceptance:**
  - [x] A sanctioned profile is refused with a localized message and expiry. Lifting the sanction restores access without a restart. Done: career schema 5 (`career_sanctions`), read at sign-in and with every 10-second access refresh (`CareerStore::access`). An account ban refuses the game sign-in (`error_code` `account_banned`, the ban's end in `CareerView.sanctions`), every website session and new website sign-in (`403 account_banned`); matchmaking and ranked bans refuse `FindMatch` (`Failed{matchmaking_ban|ranked_ban}`). Copy with the end in en, ru and zh-Hans (`client/i18n/*/moderation.json`; Russian UI-tree test on the search screen; no visual check, the Mac's screen is locked). Lifting: `match_service::tests::sanctions_refuse_find_match_with_their_code_until_lifted`, `career_backend::tests::access_refreshes_apply_and_lift_sanctions_and_a_name_reset`, the Account API session test; a suspended client signs in again every minute.
  - [x] A `chat_mute` returns "muted until …", and the muted player's messages reach no one (server test). Done: `social::tests::a_moderated_runtime_delivers_no_muted_or_switched_off_chat` and `a_muted_sender_is_refused_with_its_end_and_spends_no_cooldown` (`SocialView.error_code` `chat_muted` and `mute` with its end; what the player sent before the mute is dropped too; reactions stay).
  - [x] Force-rename changes the handle, writes an audit row, and the next presence shows the new name. Done: `omoba-account-api rename` (a `name_reset` sanction: `Player#NNNN`, renames locked until it ends) with its `career_moderation_events` row (`career_store_sanctions_apply_expire_lift_and_audit`); the access refresh carries the new name and `party::tests::a_banned_player_leaves_presence_and_a_reset_name_shows_in_the_next_presence` shows it in the next presence.
  - [x] The kill switch stops all chat delivery (test). Done: `OMOBA_CHAT=off` per server (set in `ops/vps/ru/omoba-lobby-ru.service`, enforced by `ru_host.py`) and `omoba-account-api chat off` for every server of the database without a restart (polled every 5 s); chat is refused and held chat dropped, reactions stay (`social::tests::the_chat_switch_and_a_mute_drop_held_chat_but_keep_reactions`); clients show a localized "chat is turned off" state.
  - [x] `RUNBOOK.md` has a moderation section. Done: "Moderation (LR-15)": look up a player, apply and lift sanctions, force a rename, turn chat off; audit rows are kept at least a year (a trigger refuses deleting younger ones).
- **Depends on:** LR-01, LR-16 (a throttle on new profiles, so bans hold)

#### LR-16 — Admission and abuse limits
Area: abuse/transport · **major** · M · code (+ops nftables)
- **Now:**
  - Any unseen device key creates a profile (`career-store/src/career_store.rs:276-320`). The only limits are a 100 ms per-endpoint gate and a cap of 512 clients (`server/src/career_backend.rs:645-660`).
  - There is no per-IP limit and no cleanup of profiles that never played.
  - A client-chosen `#NNNN` tag is accepted (`career-store/migrations/postgres/002_player_handles.sql:6-10`).
  - Lobby enqueue is keyed only by profile (`server/src/match_service.rs:117-185`), so two throwaway keys can hold both arenas.
  - The UDP probe and compatibility budgets (64/s each) are global and counted before return-path validation (`server/src/public_transport.rs:19,49-74,118-129`). One source, even a spoofed one, can lock everyone out of login.
  - No firewall rules are shipped (`ops/vps/ru/README.md:41-43`).
- **Requirement:**
  - Per-source budgets (per IP and per /24 or /64) apply before the global ones, with a reserve for new sources.
  - One IP holds at most 8 endpoints or career clients.
  - New-profile creation is limited per IP per day and globally, with a durable counter in Postgres and a localized error.
  - One IP holds at most K arenas at once.
  - Repeated abandoned bot-practice allocations trigger a short cooldown.
  - A retention job deletes unlinked profiles with zero matches after 30 days.
  - `ops/vps/ru` ships nftables per-source hashlimit rules for UDP 4000 and the arena range.
- **Acceptance:**
  - [x] 1000 probes/s from one address do not stop a second address from being admitted and getting a compatibility reply (unit test). Done: `public_transport::tests::a_flooding_source_cannot_lock_out_a_second_source`; budgets in `server/src/admission.rs`.
  - [x] One IP holds at most N endpoints or career clients (unit test). Done: N = 8 (`OMOBA_LIMIT_ENDPOINTS_PER_IP`), `one_ip_holds_at_most_n_endpoints` and `one_ip_holds_at_most_n_career_clients`.
  - [x] More than K new keys from one IP in 24 h are refused, while existing keys keep working (integration test). Done: K = 10 per IP, 5000 in all (career schema 4), `career_store_limits_new_profiles_per_source_and_in_total` against PostgreSQL; the refusal is localized (`error_code`, `client/i18n/*/admission.json`).
  - [x] A second BotPractice allocation from an IP that already has one Forming or Running gets a localized refusal (unit test). Done: `arena_limit` (`OMOBA_LIMIT_ARENAS_PER_IP` = 1) and the abandon cooldown `practice_cooldown`, with `queue.failed.*` copy in en, ru and zh-Hans.
  - [x] 300 clients reconnecting after a restart all get in without "unavailable" errors (harness). Done: `harness/tests/admission.rs` (PostgreSQL, ignored by default): 300 sign in, the lobby restarts on the same port, all 300 reconnect with no error and the same profiles.
  - [x] The nftables snippet is installed, and the knobs are documented in `ops/vps/ru/README.md`. Done: `ops/vps/ru/nftables-omoba.conf` with unit `omoba-limits-ru`, installed (after `nft -c`) and checked by `ru_host.py bootstrap`/`status`, proven against the fake host; the first real load is on the RU host. Knobs: README "Admission limits".
- **Depends on:** —

#### LR-17 — Health signals, monitoring, alerts and disk retention
Area: observability · **major** · M · ops (+S code)
- **Now:**
  - No metrics, external probes, `OnFailure=` hooks or alerts exist.
  - The lobby emits only in-match `MATCH_METRIC` lines (`server/src/session.rs`), with no queue, wait, bot-fill, capacity or allocation-failure data.
  - Preflight only resolves the lobby's DNS (`scripts/release.py:1334-1365`).
  - Database connect and schema errors are swallowed by `connect_runtime(&url)).ok()` every 2 s (`server/src/career_backend.rs:1242-1244`), while the unit still shows as active.
  - `worker.log` is appended forever, and archives are never pruned (`server/src/match_pool.rs:296-312,365-370`).
  - No journald limits are shipped.
- **Requirement:** The owner learns about an outage within minutes without logging in to the host, and disk use is bounded.
- **Acceptance:**
  - [ ] The lobby logs a rate-limited database failure reason (auth, network or schema) and writes a status file with `db_connected`. A test covers the schema-mismatch path.
  - [ ] The lobby prints a structured `QUEUE_METRIC` every 60 s and on every allocation outcome:
    - online, searching and in-match counts
    - queue wait p50/p95
    - bot-fill share
    - capacity wait
    - failed allocations
  - [ ] An external probe runs every minute against the UDP compatibility check, Account API `/health/ready` and the portal, and alerts to Telegram after 2 consecutive failures. Shipped: `scripts/market_probe.py` with `ops/probe/omoba-probe@.{service,timer}` (names from `scripts/markets.py`); running it needs a probe host outside the RU host and the bot (owner, `ops/probe/README.md`).
  - [ ] Alerts exist for:
    - any unit failure (`OnFailure=`): shipped, every RU unit and, by a drop-in, every PostgreSQL cluster unit → `omoba-alert-ru@.service` (`ru_monitor.py unit-failed`)
    - free disk below 15%: shipped (`ru_monitor.py check`, every 5 minutes)
    - capacity wait longer than N min: needs `QUEUE_METRIC` (server part)
    - failed allocations above N per hour: needs `QUEUE_METRIC` (server part)
    - a missing backup (LR-03): shipped, `ru_backup.py check --alert` on the host's one bot, plus a stopped backup timer
    - stale worker heartbeats: shipped, a live worker's `status.json` older than 5 minutes while the lobby runs
  - [x] `release.py preflight` adds a UDP compatibility probe (`lobby_udp`; `--contract` compares a release's `compatibility.json`).
  - [ ] Archive retention by age or size is in place, `worker.log` is capped, and a journald drop-in sets `SystemMaxUse` and `MaxRetentionSec`. The retention of logs that contain IPs is stated in LR-04. The journald drop-in is shipped (`ops/vps/ru/journald-omoba-ru.conf`: 1 GiB, 30 days, installed by `ru_host.py bootstrap`); archive retention and the `worker.log` cap are the server part; LR-04 must name the 30 days.
- **Depends on:** LR-01

#### LR-18 — Protect progress: secure-account prompt and in-game recovery codes
Area: accounts/recovery · **major** · M · code
- **Now:**
  - Identity lives in `career_identity.json` (`client/src/career_identity.rs:23`), and Android sets `allowBackup=false` (`mobile/android/AndroidManifest.xml:11`).
  - A missing key silently creates a new profile at rating 1000 (`career-store/src/career_store.rs:303-319`).
  - In-game VK linking exists (`client/src/career_devices.rs:174-186,465-485`), but its browser leg goes through the broken portal (LR-02).
  - Recovery codes need a fresh portal session (`account-api/src/devices.rs:295-317`).
  - Nothing prompts the player to secure the account. The only reminder sits in the Supporter page, which is off in RU (`client/i18n/en/supporter.json:7`).
- **Requirement:**
  - After the first completed match, and again after the first ranked match, a dismissible Home card prompts the player to link VK ID or save a recovery code. It returns until the player does one of them or declines 3 times.
  - Profile shows the account's protection status.
  - Recovery codes can be created in-game, signed by the device key.
  - A VK-verified support recovery procedure is documented.
  - The decision on Android `allowBackup` is recorded.
- **Acceptance:**
  - [ ] A fresh install shows the prompt after its first match.
  - [ ] RU end-to-end: reinstall the APK and recover the same `profile_id` through VK ID.
  - [ ] Account API Postgres tests cover the in-game recovery-code route.
  - [ ] The README and portal say that progress follows the VK-linked account.
- **Depends on:** LR-02, LR-19

#### LR-19 — VK ID app for the RU market
Area: sign-in · **major** · S · owner
- **Now:**
  - The VK ID app does not exist yet (`docs/progress/2026-10-24-market-profiles.md:52`).
  - Without `OMOBA_VKID_CLIENT_ID`, the portal shows no VK buttons (`market-web/src/components/portal.tsx:59-61`).
- **Requirement:** Create the VK ID web app:
  - trusted redirect `https://omoba.ru/api/auth/vk/callback`
  - scope `vkid.personal_info`
  - OK and Mail.ru enabled
  - LR-04 URLs entered in the VK ID cabinet
- **Acceptance:**
  - [ ] Both env files carry the app id, and the portal shows three buttons.
  - [ ] RU runbook step 6 passes with real VK: bot match → Profile → «Войти через VK ID» → link → portal sign-in → second device.
- **Depends on:** LR-04, LR-01, LR-02

#### LR-20 — In-product feedback and bug reports
Area: feedback · **major** · M · code
- **Now:**
  - The product has no feedback or bug-report entry point, route or storage (`client/src/frontend/home.rs:42-57`; `client/src/frontend/postmatch.rs:96-100`; `account-api/src/lib.rs:439-551`; `market-web/src/components/portal.tsx:224`).
  - The README has no contact (`scripts/release.py:508`).
  - The global beta uses Discord #feedback/#bugs and GitHub issues (`landing-v2/apps/omoba/src/lib/discord-community.ts`), which the RU product cannot reach.
  - The Account API rate limiter, audit table and device-key auth can be reused.
- **Requirement:**
  - An "Отзыв / Сообщить об ошибке" entry on Home, in the pause menu and on post-match.
  - The form:
    - category: bug, idea, balance or other
    - up to 2000 characters of text
    - automatically attaches build, market, platform, GPU, locale and the last `result_id`
    - optionally attaches the log tail, with consent
  - Submission: a signed `POST /v1/feedback` to the RU Account API, rate-limited (e.g. 5 per hour), with an idempotency key and an offline queue.
  - An optional one-tap 1-5 match rating.
  - Storage in RU Postgres with a retention period.
  - The owner gets a notification (Telegram or VK bot from the RU host) and can list feedback through a CLI or CSV export.
  - A portal Feedback link, and the support e-mail in the README.
- **Acceptance:**
  - [ ] A submission from Home creates one row (version, platform, market) and one owner notification within a minute.
  - [ ] An offline submission is retried once online, with no duplicates.
  - [ ] The post-match rating is stored with `match_id` and appears in a daily summary.
  - [ ] Data stays on the RU host.
  - [ ] Rate and length limits are tested, and strings exist in en, ru and zh-Hans.
- **Depends on:** LR-01, LR-04, LR-21

#### LR-21 — RU community and support channels
Area: community/support · **major** · S · owner
- **Now:**
  - The community runs on Discord (5 channels) plus a Telegram channel marked "needs-setup" (`community/projects/omoba/project.json`; `community/destinations.json:31-36`).
  - There is no VK community, RU Telegram or support e-mail.
  - Nothing is linked from the client (`client/i18n/ru`) or the RU portal (`market-web/src/components/portal.tsx:224`; `market-web/src/lib/navigation.mjs:65-80`).
- **Requirement:**
  - A VK group with news, rules and a feedback topic.
  - A Russian Telegram channel and chat.
  - At least one moderator.
  - A support e-mail that someone answers.
  - Per-market community and support URLs in Home and Settings, the portal footer and sidebar, and the README (plumbing in LR-06). RU surfaces do not link Discord.
  - A dated launch plan committed in the community hub:
    - open-beta framing
    - seeding sessions to fill human matches
    - creator outreach
    - a weekly feedback review
- **Acceptance:**
  - [ ] The RU build shows VK and Telegram buttons, and the global build shows Discord.
  - [ ] The RU community destinations exist with language `ru`.
  - [ ] The launch plan is committed.
  - [ ] Support has an owner process.
- **Depends on:** LR-06

#### LR-22 — RU web entry point
Area: landing/portal/packages · **major** · M · code (the owner approves the copy)
- **Now:**
  - The omoba.io landing (`landing-v2/apps/omoba/src/app/layout.tsx:96`):
    - is English only
    - has null download links except macOS (`src/lib/beta-access.ts:47-93`)
    - says "A public matchmaking server is not available yet" (`section-beta.tsx:180-184`) and "five hero classes" (`app/page.tsx:17`)
    - has no link to `omoba.ru`
    - links crypto `/donate` (Ekza) from the header, footer and sitemap (`site-header.tsx:57-60,94-97`; `site-footer.tsx:51`)
    - loads Vercel and Supabase analytics (`layout.tsx:143-144`)
  - The RU portal (`market-web`):
    - is set to noindex
    - falls back to omoba.io for its site link (`market-web/src/app/layout.tsx:23`; `navigation.mjs:71-77`)
    - sends signed-in users to the `/workshop` prototype (`market-web/src/app/[[...path]]/page.tsx:9`)
    - shows raw `test_status` and sizes in bytes (`portal.tsx:146`)
  - The package README is an English "playtest build" that ignores the market (`scripts/release.py:508-583`).
  - The RU Linux tarball ships `omoba-account-api` (`scripts/release.py:129,808-809`).
- **Requirement:**
  - A Russian landing (`omoba.ru`, section 7), hosted in Russia, with no foreign analytics. It:
    - makes accurate claims: 16 classes, the Verdant map, and the modes from LR-08
    - has download buttons from the RU `releases.json`, with SHA-256
    - links the community, the legal documents and the age sign
  - No donate, wallet, crypto or Ekza content on any RU surface.
  - On the RU portal:
    - `/` sends signed-in users to `/me`
    - the workshop is hidden or labelled experimental
    - sizes show in МБ and the status is localized
    - `OMOBA_SITE_URL` points to the RU landing
  - A Russian README for the RU market covering install, VK sign-in, feedback and support, legal links, the age sign and system requirements. Hosting notes are removed from player packages.
- **Acceptance:**
  - [ ] The RU landing is indexed (Yandex Webmaster) with ru hreflang, and its claims match the current RU release. Code done (2026-10-24): omoba-web's RU guest home is indexable in production with a canonical link and `hreflang="ru"`; it counts the hero classes from the game's catalog snapshot, names the map and the LR-08 modes as the game does and lists the RU catalog's downloads. Needs the RU host, then the owner adds the site to Yandex Webmaster.
  - [x] RU landing and portal tests assert that `/donate`, wallets, "Ekza" and "crypto" are absent. omoba-web `tests/ru-landing.e2e.mjs` (ten RU pages, text and links) and `tests/landing.test.mjs`.
  - [x] With `OMOBA_MARKET=ru`, `/` renders the guest home when signed out and `/me` when signed in. `tests/ru-landing.e2e.mjs`.
  - [x] `scripts/test_release.py` asserts the RU README is in Russian and includes the support e-mail, policy URLs and age sign. The real support e-mail waits for the owner (LR-21); until then `build` prints a note and the README names the portal.
  - [x] The RU Linux player tarball has no `omoba-account-api`. The RU servers are the separate bundle `release.py build server --market ru` (`ops/vps/ru/README.md#server-bundle`), the only file `ru_host.py ship` accepts.
  - [x] The global landing and README are unchanged. The omoba.io landing is not touched; the global and dev READMEs are pinned byte for byte in `scripts/test_release.py`.
  - [x] A public `/delete` page (section 7, rules for LR-22; O-15). Code done (2026-10-25): omoba-web serves `omoba.ru/delete` without sign-in, in ru, en and zh, from the portal's legal table (portal-only `delete`, like `offer`; global and cn 404). It describes the manual e-mail procedure of the privacy draft (subject, nickname with tag, owner check, device keys refused once confirmed, erasure within 30 days, anonymised results); the address is a placeholder until LR-21. `tests/ru-landing.e2e.mjs`, `tests/market-profiles.e2e.mjs`, `tests/legal.test.mjs`. Needs owner: the support e-mail, counsel's check with the policy (LR-04), the URL in Play's Data safety form (LR-69).
- **Depends on:** LR-04, LR-07, LR-08, LR-21, LR-23

#### LR-23 — RU-hosted downloads and public release labels
Area: distribution · **major** · S · ops
- **Now:**
  - Packages are always pinned to Pinata IPFS, with the gateway defaulting to `https://ekza.mypinata.cloud` (`scripts/release.py:1266-1320,1542`).
  - `test_status: 'playtest'` is hard-coded (`:1401-1403`).
  - There is no mirror in Russia and no download count.
- **Requirement:**
  - The primary download host is in Russia: a Caddy `file_server` on the RU VPS or RU object storage. IPFS becomes a mirror.
  - The status label is chosen by the owner (e.g. "Открытая бета").
  - Downloads are counted per platform.
- **Acceptance:**
  - [ ] `releases.json` URLs are on `omoba.ru` and their SHA-256 values verify. Code done (2026-10-24): `release.py publish --market ru` uploads to `omoba.ru/downloads/v<version>/` (Caddy on the RU portal site) and reads every package back; `catalog` refuses a URL off the host and adds the IPFS copy as `mirror_url`; `verify-downloads` re-checks what the portal lists. Needs the RU host for the first real run.
  - [ ] The portal shows the chosen status. The catalog carries «Открытая бета» (`scripts/markets.py` `release_label`) as `test_status`, which the portal's download page already prints; needs the host to see it.
  - [ ] Download counts per platform are available. Code done: Caddy logs only download requests (shortened addresses, no headers, 90 days) and `ru_host.py downloads` prints totals per platform, version and day (`ru_downloads.py`). Needs the host.
  - [ ] A test download from a Russian network is recorded. Procedure in `ops/vps/ru/README.md#downloads` and release checklist S11; needs the host and a Russian connection.
- **Depends on:** LR-01

#### LR-24 — Android release build and durable signing key
Area: Android distribution · **major** · S · code + owner (key custody)
- **Now:**
  - The APK is signed with a generated playtest keystore: alias `androiddebugkey`, password `android`, CN=Omoba Playtest (`scripts/release.py:962-976,1008`).
  - The native library is built without `--release`, and the file is named `-debug.apk` (`mobile/android/build.py:83,194,237-239,278`).
  - The client crate builds at `opt-level=1` (`Cargo.toml:32-36`).
  - Switching keys after launch forces every sideloaded player to uninstall, which loses their local identity.
- **Requirement:** Before the first public APK:
  - a dedicated release keystore with a strong secret, kept in CI secrets and escrowed (LR-03)
  - a release-profile native library
  - no `-debug` in the file name
  - a format RuStore accepts
- **Acceptance:**
  - [ ] `apksigner verify --print-certs` shows the production certificate, and its fingerprint is in RUNBOOK.
  - [ ] `release.py` refuses a non-dev release built with the playtest key or the default password.
  - [ ] The library comes from `target/<abi>/release`.
  - [ ] Version N+1 installs over version N.
- **Depends on:** —

#### LR-25 — Android device validation and unsupported-device message
Area: platform · **major** · M · code
- **Now:**
  - Android is an RU platform (`docs/markets.md:21`), but the game has never been run on a physical device or an emulator (`docs/progress/2026-10-10-mobile-distribution.md:41-60`).
  - The Android 12 crash is unresolved (`docs/progress/2026-09-27-android-launch-crash.md:38-39`).
  - The build requires Vulkan 1 with no GLES fallback (`mobile/android/AndroidManifest.xml:8`), and a failure can be a silent return to the home screen (`mobile/README.md:109-110,132-141`).
  - Android has no controller support (`docs/controller.md:14`).
- **Requirement:**
  - Run a device matrix.
  - Show a clear "device not supported" screen when adapter creation fails.
  - Publish minimum requirements.
  - Decide on a GLES fallback (O-11).
- **Acceptance:**
  - [ ] At least 5 physical devices popular in Russia (low, mid and high tier; Mali and Adreno; at least one low-end) each:
    - install and launch
    - complete a full bot match on touch without a crash
    - have FPS and `make android-diagnose` output recorded in a progress note
  - [ ] An adapter failure shows a localized message instead of exiting silently.
  - [ ] Minimum Android version, Vulkan and RAM are published on the portal and in the store listing.
- **Depends on:** LR-24

#### LR-67 — RU donations through YooKassa with tiered hero auras
Area: monetization · **launch scope** (owner request 2026-10-24) · L · code + owner
- **Now:**
  - Hero Supporter exists for the global market only: Solana USDC on the website and Apple StoreKit, a 30-day grant, and three auras (Solar, Lunar, Verdant) that any active grant unlocks (`docs/supporter.md`; `account-api/src/supporter.rs`, `supporter/solana.rs`; `career-store/src/career_store.rs` `supporter_status`; `server/src/career_backend.rs` `authorized_supporter_aura`; `client/src/supporter.rs`).
  - In RU the `supporter` feature is off: the client registers neither Supporter plugin (so other players' auras are not drawn either), the Account API serves no supporter route and refuses billing settings, the portal hides the pages (`docs/progress/2026-10-23-market-gating.md`).
  - `AuraStyle` is a Strict wire enum (`shared/src/protocol/wire_enums.rs`); grants always expire (`valid_until` NOT NULL, 400-day cap); no grant carries an amount; no YooKassa code exists in the repositories.
  - A complete, battle-tested YooKassa integration exists in the owner's vibe-piano project (one-time payments, «Чеки от ЮKassa», webhook re-reads, 23-hour idempotent replay, refunds); its rules are listed below.
- **Model (owner decisions 2026-10-24):**
  - Russian players donate on the RU portal: tiers **100, 250, 500, 1000, 2500, 5000, 10000 ₽** or a custom amount (min 100 ₽, max 100 000 ₽).
  - Each successful donation is a grant valid for **30 days** from payment, recorded with its amount in kopecks and its **tier points** (1 point per whole ruble paid: `amount_minor / 100` rounded down, recomputed after partial refunds).
  - The player's tier is the **sum of tier points of the grants whose 30-day window covers now** ("monthly donation"): ≥100 **Rose** (pink), ≥250 **Jade** (green), ≥500 **Azure** (blue), ≥1000 **Crimson** (red), ≥2500 **Prism I** (rainbow shimmer), ≥5000 **Prism II** (faster shimmer, sparkles), ≥10000 **Prism III** (fastest shimmer, crown ring). Names are working names; the owner may rename them. A player paying 100 ₽ every month stays Rose; 1000 ₽ every month stays Crimson. Points are provider-neutral: in M0 only YooKassa grants carry points; Apple tier products add fixed points later (LR-69); Solana grants carry 0 points unless O-27 decides otherwise.
  - The player wears any style up to the current tier, or hides the aura. Every player in a match sees it; bots never wear one.
  - Solar, Lunar and Verdant stay base styles unlocked by any active grant of any provider (unchanged rule); the tier styles need points. Global Apple and Solana are unchanged in M0; LR-69 replaces the monthly Apple product.
- **Requirement:**
  - **Market:** RU `supporter` feature on, with the payment provider list `[yookassa]`; global `[apple, solana]`. The provider list is a market property in `shared/src/market.rs`, `scripts/markets.py` and the portal `market-profile.mjs`. The Account API refuses YooKassa settings in global builds and Apple/Solana settings in RU builds. RU clients register `SupporterPlugin` (panel and world rendering) but no StoreKit bridge. The provider list becomes per region × channel in LR-68/LR-69. Until LR-69 the RU Account API keeps refusing Apple and Solana settings.
  - **Wire:** `AuraStyle` gains the seven tier styles with a protocol bump to 12 (policy in `wire_enums.rs`); `PROTOCOL_VERSION`, golden tests and `docs/release-compatibility.md` updated. Client and server ship together (section 7, O-12). `SupporterGrantSummary` gains `tier_points`; `SupporterStatus` gains `tier_points` and `tier_until` (end of the current tier from a sweep over active grants), in the same protocol 12.
  - **Account API (portal migration 009):** provider `yookassa` in the supporter CHECKs; `amount_minor` on grants (NULL for other providers) and a provider-neutral `tier_points integer NOT NULL DEFAULT 0`, set only by the Account API from the provider rule (YooKassa: whole rubles after refunds); existing rows keep 0 (no environment-driven backfill); a durable attempt table (request key, provider idempotence key persisted before the POST, provider payment id, amount, RUB, frozen receipt snapshot, accepted offer version, status `created|charging|submitted|succeeded|canceled|failed`, lease, timestamps) and a refund table; grants for the new tables in `ops/grants.sql`.
    - `POST /v1/me/supporter/yookassa/checkout` `{amount_rub, receipt_email, quote, offer_version}` with an `Idempotency-Key` → `{confirmation_url}` (HTTPS only, no credentials).
    - `GET /v1/me/supporter/yookassa/payments` (recent, owner-scoped).
    - `POST /v1/supporter/yookassa/notifications` (public, through the RU ingress): never trusts the body, re-reads the payment or refund with the shop credentials, acknowledges unknown objects with 200 without an API call, answers 5xx on processing errors so YooKassa retries.
    - A reconciliation loop (next to the Apple one) polls `submitted` attempts with queue rotation, replays lost creates with the **same** key while younger than 23 h (older → manual review, never a new POST), and scans refunds with a durable cursor; new charges stop when the refund scan is stale.
    - Settlement is exactly-once in one transaction through `apply_event_tx`: success grants 30 days with the amount; a late `canceled` never downgrades a success; a refund (full or partial) reduces or revokes that grant and the tier is recomputed; a refund that overtakes its payment settles the payment first.
    - 54-FZ: «Чеки от ЮKassa»: `receipt.customer.email` required, one item, `payment_subject: service`, `payment_mode: full_payment`, `vat_code` from `OMOBA_YOOKASSA_VAT_CODE` (1–12; 1 = без НДС), description «OMOBA — поддержка проекта (аура на 30 дней)».
    - Env: `OMOBA_YOOKASSA_SHOP_ID`, `OMOBA_YOOKASSA_SECRET_KEY`, `OMOBA_YOOKASSA_VAT_CODE`, `OMOBA_YOOKASSA_NEW_CHARGES` (exactly `1` enables new charges; reconciliation always runs), a loopback test stand-in origin for tests only. Return URL `<portal>/supporter?paid=1`.
    - Never log provider bodies (card/customer data), only `x-request-id`; amounts in kopecks without floats; the quote the browser shows must equal the server's (409 `reload` otherwise).
  - **Entitlement:** the tier and the allowed styles come from one function: `shared::supporter::SupporterTier::for_points` plus SQL `portal.supporter_points(profile, at)`. The base styles need any active grant; each tier style needs its points. The Account API dashboard, `equip`, the career store and the server's `authorized_supporter_aura` use it, and the server re-checks the equipped style every refresh. After a first grant, the default aura is the highest unlocked style, by one rule shared by `apply_event_tx` and `career_store` (both currently hard-code `'solar'`), with one row bound (64) in both readers. `months_supported` counts distinct calendar months with a donation for YooKassa grants.
  - **Client:** new palettes for the four solid tiers; the Prism tiers animate hue per frame on their shared material (all wearers of one style shimmer in sync), with intensity growing per tier; 2D fallback colours; the aura budget (23 elements) stays, the Prism III crown counts inside it; the Supporter panel shows the step-down (tier until date, then the next tier), the unlocked styles and the purchase surface returned by `payment_surface()` (constant in M0: `direct` → «Поддержать на сайте», which opens `<portal>/supporter`); the StoreKit bridge compiles only for `target_os = "ios"`; aura names and panel strings in en, ru, zh-Hans.
  - **Portal (RU):** `/supporter` with the tier cards (colour swatch and CSS preview, rainbow for Prism), a custom amount, the receipt e-mail, an offer checkbox linking `/offer`, the redirect, the return state («Проверяем оплату…», poll `me/supporter` 5 × 1.5 s, «Не оплачивайте повторно»), current tier and history; no Solana or Apple copy; prices and the offer readable without sign-in; `/supporters` recognition enabled in RU. Supporter navigation, `/supporter` and `/offer` links sit behind one switch (used later by `?app=<channel>`). A public `/delete` page describes the deletion procedure.
- **Acceptance:**
  - [ ] Unit tests: tier thresholds and the 30-day window sum (incl. expiry and partial refunds); custom amount bounds; protocol 12 golden tests; market provider lists (RU yookassa only, global unchanged); points: 1 ₽ = 1 point, partial refund recomputes points, base styles need any active grant, tier styles need points; the pinned `describe` line.
  - [x] Account API Postgres tests against a loopback YooKassa stand-in: checkout persists the key before the POST; lost-create replay under 23 h reuses the key, over 23 h goes to manual review; webhook with unknown id makes no API call; duplicate `succeeded` grants once; `canceled` after `succeeded` is ignored; refund before payment; partial and full refunds recompute the tier; the restricted portal role works (LR-67b: `account-api/tests/yookassa_provider_postgres.rs`, global and RU builds).
  - [ ] Server/harness: a player with a 1000 ₽ donation equips Crimson; equipping Prism I is refused; a second client sees the aura on that hero; after expiry the aura disappears within one refresh.
  - [ ] RU client: `SupporterPlugin` registered, StoreKit not; panel opens the portal and reads `payment_surface()`; other players' auras render (UI-tree test; visual capture when the screen is available).
  - [ ] Portal unit and e2e tests (YooKassa stand-in): all tiers and a custom amount, quote mismatch → reload, return polling, no crypto/Solana/Apple on RU pages, global unchanged.
  - [x] Docs: `docs/supporter.md`, `docs/markets.md`, `account-api/README.md`, OpenAPI, the RU runbook (shop settings, webhook URL `https://account.omoba.ru/v1/supporter/yookassa/notifications`, test shop), Caddy native list, ADR-024 (LR-67b; the webhook has its own `@yookassa` handle in `Caddyfile.ru`, not the native list).
- **Owner:** a YooKassa shop for an ИП or ООО (receipts for самозанятые are not covered by this design), «Чеки от ЮKassa» enabled, the VAT code confirmed, the webhook URL and events (`payment.succeeded`, `payment.canceled`, `payment.waiting_for_capture`, `refund.succeeded`) registered, a test shop for staging; a public offer (оферта) for supporter cosmetics with refund terms and the seller's requisites (ФИО, ИНН, ОГРНИП, address, e-mail, phone) published at `/offer` (draft from LR-06); a real test payment that checks the fiscal receipt before announcing.
- **Depends on:** LR-01, LR-02, LR-04/LR-06 (offer), LR-14 (protocol bump ships with the update path)

#### LR-68 — One app per platform; the account's region chooses servers, sign-in, data and payments

**2026-10-28 delivery:** the routing foundation is implemented in 0.45.0:
`shared::region`, first confirmation, remembered account binding, isolated keys,
raw-server UI removal, runtime feature/provider selection and blocking cold
switching. This does not close all R0–R3 criteria. See the
[current delivery and remaining gates](ru-release-2026-10-28.md). The implementation
uses `account_region.json`, retains regional preferences for now and keeps legacy
explicit config roots compatible. RU dev now has its own regional authority
(`*.dev.omoba.ru`) rather than using the global dev API: the new account-region
reply must agree with the backend that owns the identity. Provisioning it is an
ops gate; it never falls back to global.
Area: architecture/accounts/distribution · **blocker for the first unified store release** (R0 is inside M0) · L · code + ops + owner
- **Now:**
  - **Region is a compile-time build axis.**
    - `MARKET` and `FEATURES` are constants (`shared/src/market.rs`; ADR-003).
    - Plugin sets are fixed from `FEATURES` when the App is built (`client/src/plugins.rs:135,186,214-221`).
    - The settings folder comes from the const market (`client/src/platform.rs:115` `config_directory_name(shared::market::MARKET, …)`).
    - Phones cannot relaunch the process (`platform.rs:177` `CAN_RELAUNCH`).
  - **Region-scoped state is lazy and global.**
    - `connection_profile::active()` is `ACTIVE.get_or_init(..)` on a `static OnceLock` (`client/src/connection_profile.rs:515,528-529`). The first read locks in whatever it sees.
    - `main` runs `connection_profile::install()`, `passport::initialize()` and `passport::initialize_store()` before `App::new()` (`client/src/lib.rs`).
  - **Endpoints come from two systems.**
    - The lobby, Registry and passport come from `ConnectionProfile`. The global lobby is the IP literal `77.246.105.57:4000` (`client/src/session_config.rs:34`).
    - The Account API comes from `WebAccountApi::from_env` (`passport/src/web_account.rs`). The passport crate calls it directly (`passport/src/device_account.rs:13`).
    - `passport` depends on `shared`, not on `client`.
    - The Ekza SDK is an external crate with its own HTTP feature (`Cargo.toml:27`; `passport/Cargo.toml:8`).
    - The dev contour is pinned to `*.dev.omoba.io` (`scripts/contours.py:36-41`; `validate_contour_endpoints` in `web_account.rs`).
  - **The international region has no account backend.** Global production runs only `migrate`, and the portal answers `503 account_service_disabled` (`docs/architecture/system.md:181,198,201`).
  - **iOS and Android identities.**
    - iOS is global-only, and only on TestFlight (`docs/markets.md:161`; `mobile/ios/README.md`).
    - The production phone identity is `space.ekza.omoba.beta` ("Omoba Beta", `scripts/contours.py:6`).
    - RU Android uses `space.ekza.omoba.ru` (`scripts/markets.py`).
    - The APK is signed with a public debug keystore (LR-24).
  - **Sign-in.**
    - VK ID, OK and Mail.ru only, through the website `/connect` (`client/src/career_devices.rs:1-8,24`; ADR-004).
    - `social_identities.provider` allows only `'vk'` (`account-api/migrations/007_social_sign_in.sql`).
    - iOS opens Safari (`mobile/ios/BrowserBridge.swift`), and `Info.plist` registers no URL scheme.
    - The account row has no unlink action (`client/src/career.rs:1707-1729`).
  - **Device key.** It is a file in the region folder (`client/src/career_identity.rs:23`), so it is deleted when the iOS app is uninstalled.
  - **Existing data.** No RU-contour data exists (LR-01 is open). Russian citizens' career data (device keys, nicknames, match history) may already exist on the global contour, whose host location is unrecorded (O-30).
- **Model (owner decisions 2026-10-24):**
  - **One app per platform.** One production app per platform (App Store, Google Play, RuStore, direct Android and desktop) serves both regions.
  - **Two regions.**
    - `ru`: `https://omoba.ru`, `https://account.omoba.ru`, `lobby.omoba.ru:4000`. All data stays in Russia.
    - `global` ("International"): `https://app.omoba.io`, `https://account.omoba.io`, and a lobby DNS name fixed in O-31 (proposed `lobby.omoba.io:4000`).
  - **The account belongs to its region.** It is never merged or replicated across regions.
  - **Sign-in.** RU uses the omoba.ru website (VK ID, OK, Mail.ru) next to the first-party options. International uses Sign in with Apple (R2b). Apple sign-in is never offered in RU.
  - **Ekza** stays off in RU and is added later, one feature at a time.
  - **Payments** depend on region × channel. iOS never links or steers to web payments (LR-69).
- **Requirement:**
  - **Region profile (`shared`).**
    - `shared/src/region.rs` holds `RegionProfile`: `id`, `portal`, `account_api`, `lobby`, `zones`, `sign_in`, legal URLs (LR-06), `default_locale`, `features`, `storage_suffix`, `data_residency`. It also holds one `const REGIONS` table.
    - It provides `bind(id)` (once per process, `OnceLock`), `active() -> Option<&RegionProfile>` and `has(Feature)`. All of these are readable from `client` and `passport`.
    - `Market` stays as an alias for one release. `Features` gains `apple_sign_in`: global on, RU off, CN off. `scripts/markets.py` `FEATURES`, the portal `FEATURES` and the parity tests change in the same commit.
    - `shared/regions.json` is generated from the table.
      - `scripts/markets.py` reads it.
      - omoba-web vendors a copy at `src/lib/regions.json`. A sync script records the omoba-bevy commit and the SHA-256, and release checks the pin on both sides.
      - Packaging-only fields (Android identity, `blocked` reason) stay in Python.
  - **Build channel.**
    - `OMOBA_CHANNEL = app_store | google_play | rustore | direct` is set at compile time by the packager. The region never changes it.
    - Every production client carries `{global, ru}`; `cn` stays blocked.
    - Server, Account API and portal processes keep the compile-time market: one deployment serves one region.
    - The RU-locked client build (`OMOBA_MARKET=ru`) binds `ru` at startup with no picker, for M0, CI and tests, until R3.
  - **Binding (client).**
    - **Union of plugins.** The App registers every plugin any region uses. Each region-specific system runs under `run_if(region::has(Feature::…))` and `run_if(region::bound_and_accepted)`. This amends ADR-005: in the unified client, region gates are run conditions, not plugin removal.
    - **Plugin test.** A test lists the systems of the Ekza, Supporter, StoreKit and VK plugins and checks that each carries its gate.
    - **Bind-time setup.** `connection_profile::install`, `passport::initialize`, `initialize_store`, `poll_account` and every Ekza SDK client are built in `region::on_bound()`, and Ekza clients only when `has(Ekza)`.
    - **No lazy accessors.** `get_or_init` accessors are replaced by values set at bind time. They return `None` before bind, and a debug assertion fails on a region read before bind.
    - **Folders.** `platform::preferences_directory` takes the bound region. Device-level paths use a separate accessor.
    - **`ekza://` source.** It is always registered and resolves its root through `region::active()` at read time. It returns NotFound when the region has no Ekza.
    - **Lint.** A source lint forbids `shared::market::{MARKET, FEATURES}` reads in `client/` outside the region module, and `WebAccountApi::from_env` outside the region-aware constructor.
  - **First run.**
    - **Order:** language → one region card with its legal links and acceptance checkbox (LR-06) → age sign (LR-07) → Home.
    - **One Confirm** binds the region in-process (this is the first bind, not a switch). Nothing runs before it: no socket, no DNS lookup, no HTTP request.
    - **Suggestion.** The device region RU, or a Russian time zone, suggests `ru`; otherwise `global`. The StoreKit storefront is not read before Confirm.
    - **Card texts.** They describe servers and language, not residence (O-32). They are channel-aware.
      - «Россия»: «Русскоязычные серверы в России · данные хранятся в России · вход через VK ID, OK, Mail.ru».
      - "International": "Servers outside Russia · omoba.io". "Sign in with Apple" is added only on iOS, from R2b, and only when not gated.
      - Footnote: each region keeps its own profile, friends and progress.
  - **Switching.**
    - Settings → Region writes `pending` to `region.json` after a confirmation.
    - Desktop relaunches (`platform::relaunch`).
    - Phones show a full-screen "Restart required" state that cannot be dismissed. It stops all region traffic: network plugins run only while `!switch_pending`. The new region binds on the next cold start.
    - **Blocking.** The switch is blocked while StoreKit holds unfinished catalog transactions of the current region ("Finish the pending purchase first").
    - Nothing is copied or merged. The other region's profile stays on disk.
  - **Folders.**
    - **Device root** `omoba-bevy-device[-contour]`. It holds `region.json` (`{region, pending, chosen_at, apple_tokens: {token: region}, schema}`) and device preferences (language, graphics, audio, controls). `saved_language()` reads the device root.
    - **Region folders** keep their names: `omoba-bevy`, `omoba-bevy-ru` and their `-dev` forms. They hold the identity, enrollments, connection profile, Ekza store, match resume, profile card, legal acceptance and catalog cache.
    - **Override.** `OMOBA_CLIENT_CONFIG_DIR` is the device root, and region folders sit under it.
    - **Preferences migration.** On first launch `client_preferences.json` is split: device fields go to the root. When both region folders have preferences, the most recently modified file wins.
    - **Profile migration.** If exactly one region folder exists, that region is preselected and the card shows "Your profile <nick> is here". If both exist, both are shown and nothing is preselected.
  - **iOS identity.** The device key is also stored in the Keychain (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, no iCloud sync), one entry per region and contour.
    - A reinstall restores the profile from the Keychain and preselects that region.
    - Apple does not guarantee that Keychain items survive uninstall, so LR-69's restore screen still covers a lost key.
  - **Endpoints and host policy.**
    - **Endpoint owner.** One `ConnectionProfile` per (region, contour) owns the lobby, Account API, portal and, for global only, the Registry, passport and Ekza gateways. `WebAccountApi` is built from `shared::region::active()`. Environment overrides are desktop-only and must pass the policy.
    - **Policy table** (`validate_contour_endpoints` is generalized to (region, contour)):
      - (`ru`, prod): allow `omoba.ru` and subdomains, plus loopback and private LAN (desktop only). Deny everything else, including `omoba.io`, `ekza.io` and `77.246.105.57`.
      - (`global`, prod): allow `omoba.io`, `ekza.io` and the Ekza asset/IPFS gateway hosts (enumerated in R1, for example `*.mypinata.cloud`). Deny `omoba.ru`.
      - (`global`, dev): `dev.omoba.io` and `dev.ekza.io`; (`ru`, dev): its own `dev.omoba.ru` authority. Changed with the 2026-10-28 routing foundation so the authenticated account region agrees with its backend.
    - **Where it is enforced:**
      - profile resolution;
      - `WebAccountApi::new`;
      - lobby and match-allocation addresses (a bare IP is accepted only if it equals the resolved lobby address);
      - the Registry and passport setters;
      - the release catalog and feedback (LR-14, LR-20);
      - `platform::open_external_url`.
    - **One builder.** Every `reqwest` client in this repository is built through one policy builder with a resolver that refuses names outside the policy. Redirects stay disabled.
    - **Ekza SDK.** The SDK's clients cannot take the resolver. In RU they are never constructed, and the network audit proves it.
    - **Names, not IPs.** The global lobby moves from the IP literal to its DNS name (O-31; game-directory D0). Production match workers announce a DNS name in the region zone. The D1 directory is served per region.
  - **Sign-in, RU (all channels).**
    - **Protocol unchanged:** enrollment → `<region.portal>/connect?code=` → VK ID, OK or Mail.ru → `omoba.social.link.v1` (ADR-004). Labels stay as they are (`devices.button.vk`, `devices.vk.*`).
    - **Equal options (Guideline 4.8).** The RU sign-in sheet shows three options with equal prominence:
      - «Войти через VK ID, OK или Mail.ru» (website);
      - «Ввести код восстановления» (LR-18);
      - «Связать с другим устройством» (pairing).
      The last two collect no personal data.
    - **`app_store` channel.** The website opens in `ASWebAuthenticationSession` with `prefersEphemeralWebBrowserSession = true` and the callback scheme `omoba-auth` (registered in `Info.plist` `CFBundleURLTypes`).
      - `/connect?code=…&app=app_store` ends by redirecting to `omoba-auth://connect/done?status=ok|cancel`.
      - Cancel and success are handled separately, and the game still confirms through the enrollment poll.
      - The store-mode flag travels in the URL, plus a short-lived cookie with `Path=/connect`. It never sits in the portal session.
  - **Linked sign-in and unlink (Guideline 5.1.1(v); all regions and channels).**
    - Profile → Devices gets a "Linked sign-in" row that lists the linked VK, OK, Mail.ru or Apple identities.
    - «Отвязать / Unlink» sends a signed `POST /v1/me/social/unlink` (domain `omoba.social.unlink.v1`) to the region's Account API.
    - For Apple, the server also calls Apple's revoke-token endpoint.
  - **Sign-in, international (R2b).**
    - **Native iOS.** `ASAuthorizationAppleIDProvider` with no scopes. The nonce is random per attempt; the server stores its SHA-256 with the enrollment.
    - **Route.** The game posts the identity token and authorization code to `POST /v1/auth/social/apple`. The request is signed by the device key with domain `omoba.social.apple.v1`, which binds the origin, enrollment id, token hash and expiry.
    - **The international Account API:**
      - verifies the JWT (issuer, audience = bundle id, nonce, expiry);
      - stores only HMAC(`sub`) under `OMOBA_SOCIAL_SECRET` (portal migration 011 adds `'apple'`);
      - keeps only the sealed refresh token, for revocation;
      - follows ADR-004's link and switch rules and never creates a profile over HTTP.
    - **Web.** Sign in with Apple JS (Services ID) on `app.omoba.io`.
    - **Apple server-to-server notifications** for consent revocation and account deletion unlink the identity.
    - **Gating.** Sign in with Apple is hidden in the client, and refused on the server with `403 region_unavailable`, when:
      - the storefront is `RUS`;
      - the device region is RU;
      - the request IP is in RU according to a local GeoIP database (source, licence and update schedule recorded in the runbook).
      Only a reason code is logged.
  - **Servers and data.**
    - Each region's database holds only its own accounts. There are no cross-region leaderboards, friends or parties.
    - Apple transactions of another region, returned by Apple history calls, are dropped in memory and never written or logged (LR-69).
    - **Compatibility reply.** `ReleaseContract` gains `#[serde(default)] region: Option<String>`. There is no protocol or handshake bump, and the reply stays at or below `MAX_PROBE_BYTES`. The client shows a diagnostic for another region's lobby. Enforcement is the host policy, because the reply does not authenticate the server (`shared/src/compatibility.rs:1-2`).
    - **Friend codes.** They are displayed with a region prefix (`RU·nick#1234`). The `shared::career` parser accepts both forms. The client refuses a foreign-prefix code locally with a localized message. There is no server change.
  - **Channel purchase rules.** `payment_surface(region, channel)`:

    | Channel | `global` | `ru` |
    |---|---|---|
    | `app_store` | none (R2a); StoreKit tier products (R2b, LR-69) | StoreKit tier products (R2a, LR-69) |
    | `google_play` | none: no purchase entry, no link | link to `omoba.ru/supporter` (Play's Russia exemption; re-checked every release; O-25) |
    | `rustore` | n/a | none until O-26 |
    | `direct` | link to `app.omoba.io/supporter` once the global Account API is live | link to `omoba.ru/supporter` |

    - No cell links to another channel's payment method.
    - **Portal opened from a store build.** `/connect`, devices, privacy, terms, delete and the IAP footer pages carry `?app=<channel>` in the URL. They render without Supporter navigation, without `/supporter` and `/offer` links, and without payment-method text.
  - **Deletion.** Every store channel offers the in-app deletion request (LR-69). Each portal publishes a public `/delete` page (`omoba.ru/delete`, `app.omoba.io/delete`), which goes into Play Console's Data safety form.
  - **Release catalog.** `GET /v1/releases` per region gains `channel` and `store_url`. Store builds get their store link, never a web download link. A protocol or gameplay bump is a two-region train: both regions' servers, then `make compatibility-check` against each lobby, then store builds.
  - **Android identity and signing.**
    - One package id across Play, RuStore and direct downloads (O-21).
    - A release keystore (LR-24) is enrolled in Play App Signing as the owner's own key, so all channels share one certificate.
    - `versionCode` is monotonic across channels.
    - The M0 RU-locked APK and the unified APK replace each other under the shared id. Each keeps its data in its own region folder.
  - **Phases.**
    - **R0** (S, inside M0, no behaviour change):
      - `shared::region` with `bind` fixed to the build market;
      - `regions.json` plus the vendored portal copy;
      - one endpoint owner and the lint;
      - the `describe` line pinned to its new value.
    - **R1** (M):
      - `OMOBA_CHANNEL`;
      - union plugins with run conditions;
      - device root and `region.json`;
      - the first-run card and migration;
      - the restart-required switch;
      - the host policy for (region, contour) and the resolver;
      - the network-audit QA boot.
    - **R2a** (L; needs LR-01, LR-42 and the international deletion intake from O-31):
      - **iOS:** the RU sign-in sheet with equal options; `ASWebAuthenticationSession` with callback; the Keychain identity.
      - **Accounts:** unlink; in-app deletion on store channels; `/delete` pages.
      - **Purchases:** LR-69 for RU accounts; the Play build with the RU link.
      - **Release:** catalog `channel`; `region` in `ReleaseContract`; friend-code prefix.
      - The International region on iOS is device-key only, with no sign-in and no purchase surface.
    - **R2b** (M–L; after O-22, O-23, O-31): `account.omoba.io`; Sign in with Apple native and web; international StoreKit; the global `direct` link. A production notification URL only if counsel agrees.
    - **R3** (M, after R2a; R2b not required):
      - App Store submission of the unified app;
      - RU Android and desktop packages switch to the unified client;
      - the RU-locked build is retired as a player package;
      - RELEASING.md gets the two-region train.
- **Acceptance:**
  - [ ] **R0.**
    - Market tests pass for every `Region::ALL` value through a bound-region fixture.
    - The `regions.json` regeneration, `markets.py` parity and portal SHA-pin tests pass.
    - The lint is clean.
    - The RU-locked build prints the pinned new `describe` line, for example `ru (ekza off, supporter on, vk_sign_in on, apple_sign_in off)`.
  - [ ] **Run-condition tests.**
    - Every Ekza, Supporter, StoreKit and VK system carries its region gate.
    - A RU QA boot records that no Ekza system ran.
    - A region read before bind fails the debug assertion.
  - [ ] **Network audit** (`OMOBA_QA_NET_AUDIT`), covering the app's own DNS, HTTP and UDP:
    - nothing before Confirm;
    - in RU, only `*.omoba.ru` and loopback while visiting Home, Collection, Profile and Supporter;
    - in global, no `omoba.ru`.
    - OS framework traffic (StoreKit, AuthenticationServices) is listed as out of scope.
  - [ ] **Folder and migration tests.**
    - Folder matrix: region × contour, including (`ru`, dev).
    - Migration matrix: one folder, both, neither, preference conflict.
    - The override semantics.
    - A switch keeps both identities, and switching back restores the first.
    - Switch → background → foreground still shows the restart screen.
    - A switch with an unfinished current-region transaction is blocked.
  - [ ] **Host policy tests.**
    - RU refuses every `omoba.io`/`ekza.io` name and `77.246.105.57`.
    - Global refuses `omoba.ru`.
    - (`ru`, dev) allows only `*.dev.omoba.io`.
    - A match allocation to a foreign IP is refused.
  - [ ] **`payment_surface` tests.**
    - Matrix test: no web link on `app_store`, or on (`global`, `google_play`).
    - UI-tree tests of the Profile account row for every (region, channel) pair.
  - [ ] **iOS RU sign-in.**
    - The sheet shows website, recovery-code and pairing options with equal prominence.
    - The `ASWebAuthenticationSession` callback closes the sheet on success and on cancel.
    - After store-mode sign-in, a normal Safari visit to `omoba.ru/supporter` renders the page (portal e2e).
  - [ ] **Unlink.** A UI-tree test of the unlink row in both regions. A Postgres test: unlink removes the HMAC row and keeps the profile reachable through its device key.
  - [ ] **Sign in with Apple (R2b) Postgres tests:**
    - wrong audience, wrong nonce, expired token and replayed token are refused;
    - an unknown `sub` links to the current profile;
    - a linked `sub` switches;
    - a second Apple id on the same profile is refused;
    - the RU deployment refuses the route;
    - an RU IP is refused.
    Portal unit and e2e tests for Sign in with Apple JS.
  - [ ] **Cross-region.** Another region's lobby is diagnosed. A foreign-prefix friend code is refused. An old server without `region` is still accepted.
  - [ ] **Ingress tests per region.** RU exposes no `/v1/auth/social/apple`. Both regions expose `/v1/me/social/unlink` and `/v1/me/deletion`.
  - [ ] **Release catalog.** `channel` and `store_url` are present. Store builds never receive a web download URL.
  - [ ] **Play build string audit.** No payment link in (`global`, `google_play`).
  - [ ] **Real rounds.** Each region plays one real round on its own servers: bot match, website sign-in, second device.
  - [ ] **Docs.** The architecture edits are made. `docs/markets.md` records the iOS decision (closes LR-66).
- **Owner:** decisions O-21 to O-23, O-25, O-26, O-30 to O-32.
  - DNS for `account.omoba.io` and the global lobby name.
  - An App ID with Sign in with Apple, a Services ID and a key (R2b).
  - A release keystore and Play App Signing with your own key.
  - Android developer verification for the `direct` and `rustore` channels before the 2027 global rollout.
  - A counsel opinion on 149-FZ art. 8 part 10 and 152-FZ art. 18 part 5 before R2b.
- **Depends on:** LR-01 (RU host), LR-06 (consent), LR-18 (recovery code entry), LR-24 (signing key), LR-42 (erasure), LR-69 (R2a payments), O-31 (international deletion intake for R2a; full contour for R2b), game-directory D0

#### LR-69 — iOS tier auras as Apple in-app purchases in the profile card
Area: monetization/iOS · **blocker for the first iOS release that shows auras** · L · code + owner
- **Now:**
  - **StoreKit bridge.** It sells one monthly auto-renewing product. Swift rejects anything that is not `.autoRenewable` with a one-month period. Restore filters `Transaction.currentEntitlements` to that product (`mobile/ios/SupporterStoreKit.swift`).
  - **Account API.** It has one product (`OMOBA_APPLE_PRODUCT_ID`, `account-api/src/supporter.rs:21`). An unknown token gets 403 (`:347-364`).
  - **Reconciliation.** It walks known originals only: 16 per tick every 300 s under an advisory lock (`supporter.rs:426-434`; `account-api/src/main.rs:59`).
  - **Verifier.** It requires `'Auto-Renewable Subscription'`, `PURCHASED` and a token (`billing/apple-verifier/verified-events.mjs:8-11`). Its `event_version` is `signedDate` (`:13`), and its `event_id` hashes the whole normalized event (`:37`). Apple re-signs on every response, so each re-read inserts a new `supporter_events` row.
  - **Client queue.** An unverified receipt retries every 60 s, and the queue holds at most 16 receipts (`client/src/supporter_storekit.rs:150,181`).
  - **Entitlement.** `active` is any covering grant. The default aura is hard-coded to `'solar'` in two readers (`supporter.rs:193`; `career-store/src/career_store.rs:145`). The row limits differ: 100 and 64.
  - **Stacking is blocked** by `purchase_allowed_at` (`client/src/supporter.rs:954-966`) and by the Solana `already_supported` check.
  - **Copy.** Swift sends English-only messages, and the UI copy says "Monthly subscription. Auto-renews…".
  - **Apple documentation.** `Transaction.all` lists unfinished consumables, non-consumables and auto-renewable subscriptions, not non-renewing subscriptions. `Transaction.currentEntitlements` has only "the latest transaction for each non-renewing subscription" (https://developer.apple.com/documentation/storekit/transaction/currententitlements).
  - **Privacy manifest.** `mobile/ios/PrivacyInfo.xcprivacy` declares only the file-timestamp and boot-time API reasons. It has no collected-data types.
- **Model (owner decisions 2026-10-24):**
  - The iOS app never links or steers to web payments.
  - Every aura obtainable on the web and usable in the iOS app is also sold as an in-app purchase (3.1.3(b): "provided those items are also available as in-app purchases").
  - **RU charges.** In Russia, only an existing Apple Account balance can pay. Since 2026-04-01 the balance cannot be topped up in Russia either (https://support.apple.com/126891). This is accepted.
  - The player picks an aura in the profile card and pays a fixed price. There are no custom amounts.
- **Requirement:**
  - **Product type.** Seven **Non-Renewing Subscription** products, each worth 30 days from `purchaseDate`. There is no subscription group. The monthly product is removed from configuration and code.
  - **Catalog.**
    - `shared/src/supporter.rs` `APPLE_TIER_PRODUCTS` compiles (suffix, tier, points).
    - Product id = `<contour bundle id>.aura.<tier>.30d`. The client takes the prefix from its contour (prod per O-21, dev `space.ekza.omoba.dev`). The server takes the bundle id from its deployment configuration.
    - The verifier accepts the resulting list. Ids are never reused.

    | Tier | Points | US base price | Suffix |
    |---|---:|---:|---|
    | Rose | 100 | $0.99 | `aura.rose.30d` |
    | Jade | 250 | $2.49 | `aura.jade.30d` |
    | Azure | 500 | $4.99 | `aura.azure.30d` |
    | Crimson | 1000 | $9.99 | `aura.crimson.30d` |
    | Prism I | 2500 | $24.99 | `aura.prism1.30d` |
    | Prism II | 5000 | $49.99 | `aura.prism2.30d` |
    | Prism III | 10000 | $99.99 | `aura.prism3.30d` |

    Other storefronts use Apple's equalization, unless O-29 sets manual CIS prices. Points are never displayed.
  - **Grants and events.**
    - **One event per transaction:**
      - `period_id` = `original_transaction_id` = `transactionId`;
      - `valid_from = purchaseDate`, `valid_until = purchaseDate + 2 592 000 s`;
      - `revoked_at = revocationDate`, `renewal_enabled = false`.
    - **Fact-based identity.**
      - `event_id = 'apple:tx:' + transactionId + ':' + (revocationDate ?? 'live')`;
      - `event_version = max(purchaseDate, revocationDate ?? 0)`;
      - `signedDate` is not used, so re-reads add no rows.
    - **Points.** `tier_points` comes from the Account API's catalog, never from the verifier or the client.
    - **Validation.** `VerifiedEvent::validate` requires exactly 30 days for Apple. The provider set is `apple|solana|yookassa`.
    - **Portal migration 010** (numbered in merge order; LR-69 merges before Sign in with Apple's 011):
      - Apple audit columns `product_id`, `storefront`, `price_milli`, `currency`;
      - `supporter_apple_customers(app_account_token, anchor_transaction_id, last_polled_at)`;
      - `supporter_apple_token_tombstones(app_account_token, erased_at)`, written by LR-42 erasure.
      `PORTAL_SCHEMA` (`account-api/src/lib.rs:41`) and `ops/grants.sql` (game role: new tables, `EXECUTE` on `portal.supporter_points`) are updated with it.
  - **Tier across providers.**
    - Apple grants add their catalog points to LR-67's provider-neutral sum (`SupporterTier::for_points`, `portal.supporter_points`).
    - Any active grant unlocks Solar, Lunar and Verdant (unchanged). Apple Rose therefore covers Solana Hero for 3.1.3(b).
    - Solana stays unchanged (0 points, one active grant) unless O-27 decides otherwise.
    - `months_supported` counts distinct UTC calendar months of `valid_from` for Apple grants.
  - **Stacking.** Only the active-grant clause of `purchase_allowed_at` is removed. The storage, auth nonce, profile and error checks stay.
  - **Refunds.**
    - `REFUND`/`REVOKE`, or a refund-history hit, sets `revoked_at`. Only that grant's points go away.
    - `REFUND_REVERSED` is handled manually.
    - `CONSUMPTION_REQUEST` is ignored at launch.
  - **Verification by the account's region.**
    - **Routing.** `ApplePrepare`, `AppleVerify`, `AppleRestore` and polling go to `region.account_api`. RU runs in R2a; international from R2b.
    - **Secrets.** The bundle id, In-App Purchase key, issuer id and environment are server secrets on each region host. The RU Account API accepts Apple settings from LR-69 on.
    - **`apple_prepare`** returns `{app_account_token, products:[ids], environment}`, plus `product_id` (the first id) for one release for existing TestFlight clients.
    - **`accept_apple` outcomes:**
      - own token: credit, and the device finishes;
      - another profile in this region: credit that owner, answer `{credited_to_other_account:true, nick}`, and the device finishes;
      - a tombstoned (erased) token: `{credited_to_other_account:true, erased:true}`, and the device finishes;
      - a token unknown to this region: `200 {status:"not_this_region"}`, with no write and no log. The client drops that receipt from its retry queue without finishing it, so it never counts against the 16-receipt limit.
    - **Pending foreign transactions.** The client records `{token: region}` from `apple_prepare` in the device root. It defers foreign-region unfinished transactions without submitting them, and shows no repeated error toast.
  - **Discovery, restore and polling.**
    - **First purchases.** Device delivery (`Transaction.unfinished`, `Transaction.updates`) is the authoritative way first purchases are found. Finish only after the server commits.
    - **Restore.** «Восстановить покупки» runs `AppStore.sync()`. It then sends one known signed transaction (from `currentEntitlements` or `unfinished`) as `AppleRestore` to the region API.
      - The server calls Get Transaction History v2 (`productType=NON_RENEWABLE`, `revision` paging) and Get Refund History.
      - It credits every catalog transaction with `purchaseDate` within 31 days, each by its own token.
      - It drops foreign tokens in memory.
      - The reply lists the outcome of each transaction.
    - **Polling.** Refund history is polled once a day for every known customer whose grant `valid_until` falls in the last 120 days.
      - The batch is `ceil(due / ticks)` under the existing advisory lock.
      - The call budget is 1 call plus paging per customer per day, well under Apple's 10 req/s.
      - The lag is exported as a metric.
    - **Notifications.** No production App Store Server Notifications URL is set until R2b and counsel (O-22). If one is set later, it handles `ONE_TIME_CHARGE` and `REFUND`, and answers unknown tokens with 200 and no write.
    - **Verifier service.** It is an HTTP loopback service, `billing/apple-verifier/server.mjs`, on Apple's `SignedDataVerifier` and `AppStoreServerAPIClient` (O-28). It runs on both region hosts.
  - **StoreKit bridge.**
    - It configures the seven ids and requires `.nonRenewable`.
    - It emits `products:[{id, display_price}]` and the storefront country code.
    - It provides `purchase(product_id, token)` and `restore()`.
    - Errors are sent as codes, mapped to i18n keys in en, ru and zh-Hans.
  - **Profile card UX (iOS).**
    - **Entry.** An aura chip on the profile card, and the Profile "Aura" button, open the modal.
    - **Header.** It shows the step-down: "Crimson until 28 Oct · then Rose until 20 Nov", or "No aura".
    - **Grid.** 7 tier cards plus Solar, Lunar and Verdant: 2 columns on phones, 4 on tablets. Tapping a card previews it locally for free.
    - **Card states:**
      - Wearing: "Hide aura".
      - Unlocked: "Wear". There is no Buy on unlocked lower tiers.
      - Current top tier: "Wear" plus "Extend · price".
      - Locked: "Buy {tier} · {displayPrice}". When a cheaper product reaches this tier now, a second line offers "Reach now with {product} · price".
      - Base style: "Included with any aura". It cannot be bought.
      - Confirming: other Buy buttons are disabled.
      - Awaiting approval (Ask to Buy).
      - Not available: the product did not load.
    - **Outcome line** under every Buy. It is computed locally from the per-grant points and `valid_until` in `SupporterStatus`, for example "Prism III until 24 Nov, then Prism II until 2 Dec". It also names the receiving profile: «Для <nick> · Россия».
    - **Disclosure** under every Buy: "30-day aura · does not renew · cosmetic only · purchases in the last 30 days add up toward higher auras". No numeric balance is shown.
    - **RUS storefront.** A neutral note sits above the grid before any tap: "App Store purchases in your country are possible only from your Apple Account balance". Failure and cancel codes in RUS map to this note.
    - **Results.**
      - Toast "Credited to profile <nick>" for `credited_to_other_account`.
      - Restore result screen: "These purchases belong to profile <nick>. Use your recovery code or sign in to get them back", with a link to recovery.
    - **Footer:** Restore purchases, Terms, Privacy. These are store-mode variants with no payment-method text and no `/offer` link (LR-68).
    - **No steering text.** No hard-coded string mentions donations («пожертвование»), the website, YooKassa, Solana, web ruble tier prices or currency literals. Every price is StoreKit's `displayPrice`. `supporter.store.subscription` and the monthly copy are removed.
  - **In-app account deletion (every store channel; overrides O-15 for them).**
    - Profile → Settings → «Удалить аккаунт / Delete account», with a confirmation that names what is lost (profile, devices, aura entitlement).
    - It sends a signed `POST /v1/me/deletion` to the region. The server refuses the key at once and queues erasure under LR-42 (`Deleted#NNNN` tombstone, Apple token tombstone).
    - The app says "deleted within 30 days". Manual processing behind the request is allowed.
    - Apple sign-in tokens are revoked (R2b).
    - Payment ledger rows are kept depersonalized for the counsel's retention period.
    - International accounts in R2a use the deletion intake from O-31.
  - **App Store Connect (owner).**
    - **Prerequisites.** Gated by O-21 (identity) and O-24 (payouts) before any product is created:
      - Paid Apps Agreement, tax and banking;
      - Small Business Program;
      - EU DSA trader status (O-22). If the owner will not publish trader details, the 27 EU storefronts are excluded and `docs/markets.md` records it.
    - **Products.** The 7 products in all storefronts including Russia.
      - Names and descriptions in en, ru and zh-Hans, for example "Crimson Aura — 30 days. Cosmetic, does not renew. Purchases in the last 30 days add up."
      - One review screenshot each.
      - No subscription group; promoted IAP and offer codes off.
    - **Keys and testers.**
      - The In-App Purchase key on each region host.
      - Sandbox testers in the US and RU storefronts, including a zero-balance RU tester.
    - **App Privacy:**
      - Identifiers → User ID, linked;
      - Purchases → Purchase History;
      - Gameplay Content and Product Interaction;
      - Diagnostics and Customer Support / Other User Content once LR-20 ships on iOS.
      - No name or e-mail, and no tracking.
      `PrivacyInfo.xcprivacy` gains matching `NSPrivacyCollectedDataTypes`.
    - **Metadata.** The description, screenshots, What's New and IAP localizations pass the same no-steering audit.
  - **Review notes (draft).**
    - **Playing without login.** The game is fully playable without login. The account is a first-party device key; no name, e-mail or phone is collected.
    - **Region choice.** First launch asks for a region.
      - Russia: optional website sign-in (VK ID, OK, Mail.ru), with equal first-party options next to it (recovery code, device pairing). Sign in with Apple cannot be offered to users in Russia (149-FZ art. 8 part 10).
      - International (R2b): Sign in with Apple.
    - **Purchases.** Choose "Russia" to test in-app purchases in R2a.
    - **Auras.** Cosmetic 30-day non-renewing purchases. Every aura usable in the app is sold in the app (3.1.3(b)), with no links or calls to action toward other purchase methods.
    - **Restore.** Restore purchases is in the aura modal. Unlink is in Profile → Devices; deletion in Profile → Settings.
    - **Demo access.** A recovery code for a pre-seeded RU profile, with steps.
- **Acceptance:**
  - [ ] **Unit tests:**
    - catalog ids are unique per contour, and points equal the thresholds;
    - `for_points` boundaries;
    - step-down and outcome-line computation;
    - "cheapest reach" selection;
    - old `SupporterStatus` JSON still deserializes.
  - [ ] **Verifier tests:**
    - non-renewing is accepted;
    - auto-renewing, an unknown product, `FAMILY_SHARED` and a missing token are rejected;
    - `REFUND` revokes;
    - the same transaction with two `signedDate` values yields one `event_id`.
  - [ ] **Account API Postgres tests:**
    - Apple Rose plus YooKassa 250 ₽ gives Jade;
    - a refund lowers the tier;
    - equipping Prism I with 1000 points is refused;
    - re-polling adds no `supporter_events` row;
    - another profile's token credits its owner;
    - a tombstoned token finishes;
    - an unknown token gets `not_this_region` with no row and no log;
    - `AppleRestore` credits two same-tier purchases;
    - the RU configuration accepts Apple settings and refuses Solana;
    - `months_supported` counts distinct months;
    - `PORTAL_SCHEMA` and the grants match migration 010.
  - [ ] **Client tests.**
    - `not_this_region` and deferred foreign receipts never fill the 16-entry queue.
    - A region switch is blocked with an unfinished current-region transaction.
  - [ ] **Server/harness.** An Apple Crimson grant equips Crimson. After the refund, the aura disappears within one refresh.
  - [ ] **iOS UI-tree tests.**
    - Every card state, including base styles, Extend only on the top tier, and the outcome line.
    - The RUS note, the credited-to-other toast and the restore result screen.
    - No numeric balance.
  - [ ] **String audit** of hard-coded iOS strings, StoreKit copy, store-mode Terms and Privacy, and App Store metadata and IAP localizations (checklist).
  - [ ] **`mobile/ios/Products.storekit`** holds the 7 products.
  - [ ] **Sandbox matrix** recorded in `docs/progress/`:
    - each tier;
    - stacking (two of the same tier);
    - Ask to Buy, cancel, declined;
    - refund;
    - reinstall plus restore (Keychain key present → same profile; key lost → restore result screen);
    - a second game account on the same Apple ID;
    - RU zero-balance note;
    - a region switch with an unfinished transaction is blocked, and a deferred foreign transaction is credited after switching back.
  - [ ] **RU host.** An outbound probe to the App Store Server API succeeds (sandbox). The verifier service runs on the RU host.
  - [ ] **In-app deletion.** It blocks the key immediately. The LR-42 Postgres test leaves no rows after erasure. A Play build has both the in-app and the `/delete` path.
  - [ ] **Docs:** `docs/supporter.md`, ADR-025, `docs/markets.md`, OpenAPI, the RU runbook (Apple key, verifier service, migration of the `OMOBA_APPLE_*` variables).
- **Owner:**
  - Decisions O-21, O-24 (before products are created), O-28 and O-29.
  - O-22 (DSA trader status).
  - App Store Connect setup and submission as listed.
  - The retention period for payment rows from counsel.
  - A planned response to a 4.8 rejection: name the equal first-party options; if needed, ship RU iOS with website sign-in removed and recovery code plus pairing only.
- **Depends on:** LR-67 (migration 009, protocol 12, tier engine), LR-68 R1 (runtime region, channel), LR-01 (`account.omoba.ru`), LR-42 (erasure), LR-06 (Terms/Privacy), O-21, O-24, O-28; the international part depends on LR-68 R2b and O-31

#### Documentation changes when LR-68 and LR-69 land
The architecture pages describe what is built, so they change in the PRs that build it, not before:
- **`docs/architecture/system.md`**
  - **Build axes.** "Build axes: contour and market" becomes "Build axes: contour, channel and region":
    - clients bind a runtime region; servers keep the compile-time market;
    - add `OMOBA_CHANNEL`;
    - add the device root `omoba-bevy-device[-contour]` next to the region folders.
  - **Components › Game client:**
    - endpoints come from the bound region's connection profile (`shared::region`), and desktop overrides must pass the host policy;
    - Ekza SDK clients exist only when the region has Ekza.
  - **Communication paths.** Add:
    - Account API (each region) → App Store Server API (outbound HTTPS: history, refund history, transaction info);
    - international Account API → Apple ID (R2b: token exchange and revocation);
    - VK ID is RU only;
    - no App Store notification path until R2b.
  - **Production host (global market):**
    - planned `account.omoba.io`, the deletion intake and the lobby DNS name (O-31);
    - "host location unrecorded (O-30)".
  - **RU host:**
    - "accepts YooKassa settings, and Apple settings from LR-69; refuses Solana";
    - the Apple In-App Purchase key and the Node verifier service.
  - **Fail-closed guards.** Add the host policy per (region, contour), the policy resolver, run-condition gates, the network-audit QA boot, and the optional `region` in `ReleaseContract`.
  - **Ingress allowlists.** Add `/v1/me/deletion`, `/v1/me/social/unlink` and the Apple restore action in both regions. `/v1/auth/social/apple` is international only.
- **`docs/architecture/identity-and-accounts.md`**
  - **Principles.** Add: "An account belongs to one region; an installation holds one identity per region; regions are never merged; on iOS the device key is also kept in the Keychain."
  - **Sign in with VK ID:**
    - equal first-party options on the same sheet (Guideline 4.8);
    - iOS uses an ephemeral `ASWebAuthenticationSession` with the `omoba-auth` callback;
    - labels are the same on all channels.
  - **New section "Linked sign-in and unlink":** `omoba.social.unlink.v1`, and Apple token revocation.
  - **New section "Sign in with Apple (international, R2b)":**
    - random nonce, `omoba.social.apple.v1`, HMAC(`sub`), no scopes;
    - sealed refresh token for revocation;
    - server notifications;
    - gating.
  - **Signature domains.** Add `omoba.social.apple.v1`, `omoba.social.unlink.v1` and the deletion domain.
  - **Markets, contours and database roles:**
    - the table becomes per region;
    - Supporter: RU = YooKassa web plus Apple on iOS; global = Solana web, plus Apple on iOS from R2b;
    - the schema list includes portal 8–11.
  - **Open questions.** Close "Supporter scope in free markets". Point "Global accounts" to O-31, and the global host location to O-30.
  - **Add "Account deletion":** in-app request on store channels, `/delete` pages, LR-42 erasure, Apple token tombstones.
- **`docs/architecture/build-and-release.md`**
  - **Compile-time settings.** Add `OMOBA_CHANNEL`. `OMOBA_MARKET` now applies to server binaries, CI and RU-locked builds.
  - **Build axes.** `contour × market (Flavor)` becomes `contour × channel`; production clients carry both regions.
  - **Identities:**
    - one production identity per platform (O-21), with the dev identity kept;
    - one Android signing certificate across channels through Play App Signing with the owner's key;
    - `versionCode` monotonic across channels;
    - Android developer verification for `direct` and `rustore` before 2027.
  - **Release flow.** The two-region train; the release catalog per region carries `channel` and `store_url`.
  - **Platform builders › iOS:**
    - `Products.storekit`;
    - update `PrivacyInfo.xcprivacy` (add `NSPrivacyCollectedDataTypes`);
    - `CFBundleURLTypes` `omoba-auth`;
    - the Sign in with Apple capability (R2b).
- **`docs/architecture/protocols.md`**
  - **Current version markers.** Protocol 12 includes `SupporterGrantSummary.tier_points`, `SupporterStatus.tier_points` and `tier_until`. There is no protocol 13.
  - **§2 Compatibility preflight.** `ReleaseContract.region` is optional (`serde(default)`), diagnostic only; enforcement is the host policy.
  - **§6 Account API HTTP contract / Routes by exposure:**
    - `POST /v1/me/deletion`, `POST /v1/me/social/unlink` (native, signed);
    - `POST /v1/auth/social/apple` (native, signed, international);
    - Sign in with Apple notifications (public, international);
    - `NativeSupporterAction::AppleRestore`;
    - `apple_prepare` returns `products` (plus `product_id` for one release);
    - `accept_apple` outcomes: `credited_to_other_account`, `erased`, `not_this_region`.
  - **§9.** "Market switches" becomes "Region and channel switches", including the `payment_surface` matrix.
- **`docs/markets.md`**
  - **Title and intro:** "Regions: one app, the account's region decides".
  - **Tables.** Region rows plus a channel matrix replace the per-build platform table.
  - **Line 161** becomes: "iOS ships as one App Store app; RU accounts use it with website sign-in or first-party sign-in and StoreKit tier products (LR-68, LR-69)."
  - **Sign in with VK ID.** Same labels on all channels, with equal first-party options.
  - **Adding a market** becomes "Adding a region": `shared/src/region.rs` → `regions.json` → `markets.py` and the vendored portal copy (fixes the stale `market.mjs` pointer, decisions.md open question 7).
  - **EU storefronts.** Record their availability (O-22).
  - **RU release runbook:**
    - Apple key and verifier on the RU host;
    - Play's Russia exemption re-checked each release;
    - RU-locked packages until R3.
- **`docs/supporter.md`**
  - **Delivery status.** Tier points, seven non-renewing Apple products, the provider matrix per region × channel.
  - **Monthly Hero status.** Replace the "monthly subscription" requirement with the non-renewing tier model. `months_supported` uses distinct months for YooKassa and Apple.
  - **Payment periods and ownership:**
    - stacking allowed for Apple and YooKassa;
    - another profile's token credits its owner;
    - a refund removes only that grant's points;
    - base styles need any active grant.
  - **Apple provider boundary:**
    - catalog per contour;
    - `.nonRenewable` only;
    - fact-based `event_id`;
    - device delivery for first purchases, `AppleRestore` through history, daily refund history;
    - no production notification URL;
    - error codes.
  - **Add iOS UX rules:** no web or donation wording, `displayPrice` only, outcome line and stacking disclosure, no visible balance.

### M1 — First public weeks (self-running hardening)

#### LR-26 — Drain mode, maintenance notices and announcements
Area: maintenance · **major** · M · code+ops
- **Now:**
  - Units use `KillMode=control-group` and `TimeoutStopSec=20` (`ops/vps/ru/omoba-lobby-ru.service:31-34`), and a restart is by design a restart of the whole group (`ops/vps/README.md:9`).
  - The lobby loop handles no signals (`server/src/runtime/mod.rs:314-322`).
  - Every deploy kills all running matches, which settle Interrupted and unrated.
  - The queue and parties live in memory and are lost (`server/src/match_service.rs:26-34`; `docs/party.md:69-73`).
  - There is no maintenance or MOTD state (`shared/src/match_service.rs:28-46`).
  - Release 0.44 waited by hand for matches to end (`docs/progress/2026-10-10-release-044.md:25`).
- **Requirement:**
  - A drain flag or signal:
    - the lobby refuses new allocations with a localized maintenance message and ETA
    - running workers finish
    - SIGTERM waits up to N min
  - Workers survive a lobby restart (their own scope or slice).
  - A server notice `{kind: maintenance|update|info, starts_at, localized text, url}` appears as a Home banner and as an in-match toast 5 min ahead.
  - An announcement list the owner can edit without a release, also shown on the portal top bar.
  - RUNBOOK sections for maintenance and announcements.
- **Acceptance:**
  - [ ] Harness: a match is started, then drain is enabled; the match completes and settles; a new queue attempt gets the maintenance message.
  - [ ] A lobby restart mid-match on the dev contour lets the match complete and settle.
  - [ ] An edited notice reaches running clients within a minute.
  - [ ] The disconnect line during maintenance says "Server maintenance".
- **Depends on:** LR-01

#### LR-27 — Production deployer
Area: deployment automation · **major** · M · code
- **Now:**
  - RU install and rollback are manual (`ops/vps/ru/README.md:14-56`), and only the dev contour has a guarded deployer (`ops/vps/dev/lobby_dev.py`).
  - Migrations need a manual owner login and `grants.sql` with an explicit table list (`account-api/ops/grants.sql:1-30`).
  - The Account API requires portal schema exactly [1..7] (`account-api/src/lib.rs:41,122-131`), so a rollback fails after a forward migration.
- **Requirement:** `ops/vps/ru/ru_deploy.py`, modelled on `lobby_dev.py`:
  - target guard, upload and checksum check
  - drain or idle guard (LR-26)
  - pre-migration backup (LR-03), migrate and grants
  - unit install, including nftables (LR-16) and the journald drop-in (LR-17)
  - start-and-verify, including a compatibility probe of the lobby and a worker
  - automatic rollback and a deployment history
  - install of the release catalog
  - an additive-schema window, so the N-1 binary runs on schema N
- **Acceptance:**
  - [ ] `--check` changes nothing and prints the plan.
  - [ ] `--apply` deploys, probes and rolls back on failure.
  - [ ] The history file records release, sha256, time and result.
  - [ ] Tests cover refusals for the wrong host, running matches and a checksum mismatch.
  - [ ] An N-1 binary runs on schema N for additive migrations.
  - [ ] Grants use `ALTER DEFAULT PRIVILEGES` or are regenerated automatically.
- **Depends on:** LR-03, LR-26

#### LR-28 — Snapshot bandwidth and connection quality
Area: netcode · **major** · M (L if delta encoding is needed) · code
- **Now:**
  - Each recipient gets a full-world JSON snapshot every 50 ms (`server/src/snapshot.rs:16,39-70`).
  - Packets are split into 1200-byte fragments, and losing one fragment loses the whole snapshot (`shared/src/transport.rs:11-18`).
  - There is no compression or delta encoding.
  - 5v5 snapshots "can exceed 8 KiB" (`docs/progress/2026-09-05-3d-readiness-audit.md:74`). The resulting ~160 KB/s per player is an estimate; nobody has measured it.
  - No WAN test has been run (`docs/progress/2026-09-22-public-mvp.md:131-134`), and the HUD shows no ping or loss.
  - The MTU-safe framing should be kept.
- **Requirement:**
  - A declared per-player budget (e.g. ≤ 40 KB/s).
  - zstd compression first; delta encoding if the budget is still missed.
  - A netem WAN test.
  - Ping and loss in the HUD, with a warning.
  - The host's egress fits the plan.
- **Acceptance:**
  - [ ] Measured bytes per second, per player and per match in a 5v5 with bots, are recorded in docs together with the budget, and the budget is met.
  - [ ] At 100 ms RTT, 20 ms jitter and 2% loss (netem), snapshot-gap p95 stays under the declared threshold and no hero gets stuck.
  - [ ] The HUD shows ping and loss.
  - [ ] The owner records the host city and the monthly egress allowance.
- **Depends on:** LR-09

#### LR-29 — Crash reporting and client logs
Area: diagnostics · **major** · M · code
- **Now:**
  - Release builds on Windows hide the console (`client/src/main.rs:3`).
  - Output goes through `eprintln!` and the default LogPlugin, with no file sink (`client/src/lib.rs:91-124`).
  - There is no panic hook, crash dialog or upload. Android can be diagnosed only over adb (`mobile/README.md:143-160`).
- **Requirement:**
  - A rotating log in `settings_directory()/logs` (3 × 5 MB).
  - A panic hook writes `crash-<ts>.txt`: build label, OS, GPU, market, the last 200 log lines and the backtrace.
  - On the next start, an opt-in prompt "The game closed unexpectedly — send report?" sends the report to `POST /v1/crash`, with size and rate limits and no personal data. The answer is remembered.
  - A GPU failure shows a localized native dialog.
  - Settings offers "Open logs folder" and "Copy diagnostics".
  - Crash counts go into the LR-30 digest.
  - The README states where logs are.
- **Acceptance:**
  - [ ] A forced panic in a dev build writes a file and, after consent, stores one report.
  - [ ] Declining sends nothing and is remembered.
  - [ ] A GPU failure shows the dialog.
  - [ ] The README (ru/en) states the log path.
- **Depends on:** LR-20, LR-04

#### LR-30 — Product analytics and daily owner digest
Area: analytics · **major** · M · code
- **Now:**
  - There is no event table, dashboard or digest.
  - Queue wait lives only in memory (`server/src/match_service.rs:262`; `career-store/src/matchmaking.rs:19,130,256`).
  - `MATCH_METRIC` lines go to journald, and only a bot test harness reads them (`scripts/verify_beta_match.py`).
  - Durable facts already exist:
    - `career_matches` (`career-store/migrations/postgres/001_career.sql:18-49`)
    - `portal.player_match_facts` (`account-api/migrations/001_portal.sql:67-85`)
    - `career_keys.created_at` and `web_sessions`
  - There is no report on how humans do against bots (`docs/plans/bot-personas.md:1071`).
- **Requirement:**
  - **First,** SQL views over the existing data:
    - DAU/WAU/MAU of match players and new profiles per day
    - D1/D7/D30 retention
    - completion and abandon rates, and median duration
    - human win rate against bots per mode
  - **Then,** an append-only `analytics_events` table written by the lobby and the Account API:
    - session start and end, with a salted profile hash, platform, version and market
    - queue enter, leave and matched, with mode, party size and wait in ms
    - match start and end
    - feedback sent and report sent
  - A daily Telegram or VK digest, with alert thresholds (e.g. p95 wait > 5 min, spikes in crashes or feedback).
  - A retention period (e.g. 13 months) stated in the policy.
- **Acceptance:**
  - [ ] A Postgres integration test over a scripted day returns the correct DAU, queue-wait percentiles and completion rate.
  - [ ] Analytics rows contain no raw IP or VK id.
  - [ ] The owner gets one digest per day.
  - [ ] A weekly human-vs-bot report per mode feeds the decision on online bot difficulty (O-6).
- **Depends on:** LR-17, LR-04

#### LR-31 — Player-facing text: codes and localization for every server message
Area: localization · **major** · M · code
- **Now:**
  - Unrated reasons and abandon reasons are English prose (`server/src/career_runtime.rs:523-563,628-632,791`; `server/src/career_backend.rs:1443-1454`).
  - The client translates only snake_case codes, and only two have keys (`client/src/career.rs:1159-1168`; `client/i18n/ru/career.json:26-27`).
  - The portal shows the raw reason (`market-web/src/components/portal.tsx:89`).
  - Nickname errors are English (`shared/src/career.rs:18-30`).
  - Matchmaking failures show six raw codes (`client/src/match_service.rs:357-362`; `client/i18n/*/searching.json:20`).
  - Chat errors are English (`server/src/social.rs:140-169`; `client/src/social.rs:199-226`).
  - The default nickname pool is English in every market (`career-store/migrations/postgres/002_player_handles.sql:12-14`).
  - Bot names are "Bot N", and the `net.practice.bot` key goes unused (`server/src/career_runtime.rs:78-87`).
  - Developer text reaches players:
    - `GAME_SERVER_ADDR` in the waiting line (`client/i18n/en/net.json:4`)
    - "Server: {addr}" in Settings (`client/src/pause_menu.rs:2064-2065`)
    - `show_fps` defaults to true (`client/src/render_settings.rs:25-30`)
    - the title "Omoba Beta" (`client/src/platform.rs:24`)
    - Unix-second expiry times (`client/i18n/ru/web.json:5-6`)
    - developer lighting and model-scale sliders (`client/src/pause_menu.rs:1365-1435`)
- **Requirement:**
  - The protocol carries stable codes for unrated reasons, CareerView, draft, social, matchmaking and nickname errors. Prose is kept for old clients.
  - Client and portal dictionaries cover every code in en, ru and zh-Hans.
  - Default nickname bases are per market, or a neutral pattern.
  - Bot names are localized.
  - Production builds show no env vars, addresses, timestamps, raw codes or developer sliders. FPS is off by default, and the title is "OMOBA".
- **Acceptance:**
  - [ ] A test checks every emitted code against the client and portal dictionaries.
  - [ ] The i18n strict tests are extended to the protocol mapping.
  - [ ] A manual RU smoke run is all Russian: bot match, a match cut short, a full queue and rate-limited chat.
  - [ ] QA captures contain no snake_case codes.
- **Depends on:** LR-08

#### LR-32 — Pre-start resilience: ready check, per-player leave, dodge cooldown
Area: matchmaking ready check · **major** · L · code
- **Now:**
  - There is no accept step.
  - The draft needs the full roster, and bots spawn only after every human joins (`server/src/prematch.rs:93-107`; `server/src/bots.rs:103-108`; `server/src/match_allocation.rs:187-200`). One missing human holds the others until the 180 s join deadline, and then everyone is dropped (`server/src/match_service.rs:342,589-596`).
  - A slow loader resets the draft (`server/src/prematch.rs:157-166`).
  - Any one player's Cancel, in the draft or on the search screen while the allocation is Ready or Forming, aborts the allocation for all 10:
    - client: `client/src/frontend/draft.rs:233-258`, `client/src/net/session.rs:875-889`
    - server: `server/src/career_runtime.rs:1020-1041`, `server/src/match_service.rs:186-195`
  - There is no penalty, no re-queue and no message. Re-queue exists only for Play again (`client/src/match_service.rs:264-317`).
- **Requirement:**
  - The lobby runs a 10-15 s "Match found: Accept" step before spawning a worker.
  - Players who decline or time out are removed. The rest return to the front of the queue with their original queue age.
  - Leaving before the match starts removes only that seat.
  - An escalating dodge cooldown is persisted.
  - The remaining players see a localized message, and a cancelled allocation automatically resumes searching.
- **Acceptance:**
  - [ ] A ReadyCheck view exists, and the client shows Accept/Decline with a timer (en/ru).
  - [ ] Lobby test: 10 selected, 1 declines → 9 are re-queued in order, and no worker spawns.
  - [ ] Server test: one CancelQueue before start re-queues the other 9; the leaver's cooldown survives a reconnect and a restart.
  - [ ] Client test: a cancelled allocation shows a localized reason and resumes searching.
- **Depends on:** LR-08

#### LR-33 — Leaver and AFK policy
Area: match integrity · **major** · L · code (owner sets thresholds)
- **Now:**
  - `ParticipantResult.disconnected` exists (`shared/src/career.rs:101`), but settlement ignores it (`career-store/src/career_store.rs:613-630,635-688`). A rated leaver on the winning team still gains rating and XP.
  - No AFK detection exists: `PLAYER_TIMEOUT` counts any packet (`server/src/session.rs:29,43`), and detach happens only on a timeout or a Leave (`server/src/session_recovery.rs:44-80`).
  - There is no abandon counter, cooldown, surrender or remake.
  - The UI already labels disconnected rows (`client/i18n/ru/career.json:35`; `market-web/src/components/portal.tsx:90`).
- **Requirement:**
  - Record connected time per participant.
  - AFK means no movement or cast input for X s while alive. An AFK player is detached through the existing takeover, can reclaim on new input, and is marked as a leaver.
  - In ranked, a leaver takes a loss with a fixed rating penalty and no XP, and teammates lose less.
  - An abandon counter drives an escalating cooldown (e.g. 5 min, 30 min, 24 h) shown in searching.
  - A remake vote is available in the first M minutes.
  - A reconnect within the grace period is not an abandon.
  - Profile shows the leaver count, and the thresholds are written in `docs/match-progression.md`.
- **Acceptance:**
  - [ ] A heartbeat-only human for 90 s is flagged AFK, the hero goes to AI, and new input reclaims it.
  - [ ] A rated player disconnected for 80% of a won match gets no positive delta and no XP.
  - [ ] Two abandons in 24 h produce a localized cooldown.
  - [ ] Remake has a test, and `career_store_tests` cover the settlement policy.
- **Depends on:** LR-08, LR-11, O-13

#### LR-34 — Matches end on their own terms: surrender, bot endgame, map preset
Area: game length · **major** · M · code (+owner)
- **Now:**
  - Victory happens only when a BaseTower falls (`common/src/sim/towers.rs:75-79`), and there is no surrender.
  - Public games always use the three-tier map (`server/src/match_pool.rs:342`); the shorter presets exist only as examples (`docs/map-customization.md:9-12,30-43`).
  - Bots skip boss camps (`common/src/bots.rs:2428`).
- **Requirement:**
  - Set a target match length.
  - Humans get a team surrender vote after minute N; a surrender is a loss under the existing XP policy.
  - A time-cap adjudication or sudden-death rule.
  - Bots get macro behaviour: group, push when ahead, take objectives.
  - The owner picks the map preset for casual and bot modes.
- **Acceptance:**
  - [ ] In the simulator, ≥ 90% of Elite-vs-Elite rounds reach Victory or the cap rule within 25 min.
  - [ ] Surrender is available after the configured minute.
  - [ ] The median duration of human bot matches is reported (LR-30).
- **Depends on:** LR-11, LR-30, O-13

#### LR-35 — Ranked availability at low population
Area: ranked matchmaking · **major** · M · code (owner sets policy)
- **Now:**
  - Players are compatible only with the same preference, the same newcomer flag and a rating gap of at most 300, and the window never widens (`server/src/match_service.rs:372-376,481-531`; `shared/src/career.rs:10,176-178`).
  - A humans-only queue never launches below 10 players.
  - Searching shows humans found out of needed, elapsed time and the bot-fill countdown (`match_service.rs:247-266`), but no total population, ETA or fallback.
  - The party lobby already lists up to 16 online players (`server/src/party.rs:468-500`).
- **Requirement:**
  - Apply the O-1 policy.
  - The Ranked window widens over time (e.g. +100 per 30 s up to 600), and the store accepts the widened cohort.
  - Online, searching and in-match counts appear on Home and Searching.
  - After N min, Ranked offers a switch to Casual or vs Bots.
- **Acceptance:**
  - [ ] 10 mixed-cohort humans with a 0-900 rating spread form a Ranked match within the configured maximum wait.
  - [ ] A unit test covers the widening schedule, and the store accepts the widened cohort.
  - [ ] Home and Searching show the counts (en/ru captures).
  - [ ] A fallback button appears after the timeout.
- **Depends on:** LR-08, O-1

#### LR-36 — Casual backfill: humans replace bots
Area: bot fill · **major** · L · code
- **Now:**
  - The roster freezes at start, so new players cannot replace bots (`docs/public-mvp.md:19-23`; `server/src/bots.rs:103-108`).
  - Two Casual players arriving more than 30 s apart get separate bot arenas.
- **Requirement:**
  - A waiting Casual player can replace a bot in a forming or early-running casual arena, under a defined result policy.
  - First step, before full backfill: merge waiting players across cohorts (partly done in LR-08) and lengthen the wait while other humans are queuing.
  - Rated allocations stay frozen.
- **Acceptance:**
  - [ ] Two Casual clients queued 45 s apart share an arena while a seat is open (test).
  - [ ] Seat history stays consistent under public-casual-v1, with no double rewards.
  - [ ] Rated allocations are unchanged.
- **Depends on:** LR-08, LR-09, O-2

#### LR-37 — Ranked integrity: eligibility gate, same-IP seating, anomaly report, void-match tooling
Area: ranked integrity · **major** · L · code+ops (owner sets thresholds)
- **Now:**
  - A fresh install gets a fresh newcomer profile at 1000 (`career-store/src/career_store.rs:276-319`).
  - Ranked has no gate, and the docs disclaim smurf prevention (`docs/match-progression.md:85-91,155-156`).
  - The matchmaker rejects only duplicate ids (`career-store/src/matchmaking.rs:110-124`), with no same-IP rule.
  - Receipts are immutable, and there is no void-match path or ranked ban (`account-api/src/main.rs:13`; `server/src/bin/migrate-career.rs` only).
- **Requirement:**
  - Ranked unlocks at the owner's chosen level or match count. In RU it also needs a linked VK ID, one per profile (O-3).
  - Same-IP profiles are never on opposing teams in a rated match, and same-IP pairs are flagged.
  - A periodic anomaly report lists suspicious pairs.
  - An operator CLI writes a compensating "voided" record that reverts rating, XP and W/L in one transaction, with an audit row.
  - A ranked ban is applied through the LR-15 sanctions.
  - RUNBOOK gets a section.
- **Acceptance:**
  - [ ] Below the threshold, the player sees "Ranked unlocks at level N".
  - [ ] The RU build requires VK for Ranked (test).
  - [ ] A matchmaker test shows a same-IP pair never on opposing teams.
  - [ ] The anomaly report finds a seeded colluding pair.
  - [ ] Voiding a match restores all 10 participants exactly.
  - [ ] Every operator action writes an `audit_events` row.
- **Depends on:** LR-08, LR-15, LR-16, LR-19

#### LR-38 — Player reports
Area: reporting · **major** · M · code
- **Now:**
  - There is no report action (`client/src/frontend/postmatch.rs:96-100`; `client/src/frontend/home.rs:42-57`; `account-api/src/lib.rs:439-551`).
  - Chat is a 32-event, 30-second buffer in memory (`server/src/social.rs:1,16-18`), so a complaint cannot be checked.
- **Requirement:**
  - A Report action on the scoreboard, chat lines, post-match details and the portal player page.
  - Categories: name, chat, cheating, leaving/AFK, other.
  - The report is signed. Limits: 10 per day per reporter, and one per reporter, target and match.
  - The server attaches the target's recent chat and the match and result ids.
  - Reports are stored in RU Postgres with a retention period.
  - The owner is notified, and the CLI can list and close reports.
- **Acceptance:**
  - [ ] One report creates one row with the match id and the target's recent chat (server test).
  - [ ] A repeated report is idempotent.
  - [ ] The reporter sees a localized "Report sent", and the target is never told who reported.
  - [ ] The owner is notified.
- **Depends on:** LR-15

#### LR-39 — Block list and invite controls
Area: anti-harassment · **major** · M · code
- **Now:**
  - Mute resets every match (`client/src/social.rs:268-269,321-328`).
  - Invites have no throttle and no block (`server/src/party.rs:222-263`).
  - The online list shows everyone (`party.rs:468-500`).
  - A rejected friend request is deleted, so it can be sent again (`career-store/src/career_store.rs:921-935,948-961`).
- **Requirement:**
  - A `career_blocks` table (cap 256).
  - Blocking removes any friendship and works in both directions. It suppresses:
    - chat
    - party invites
    - friend requests (silent no-op)
    - the online-list entry
  - Block and Unblock in the friends list, chat line, lobby row and portal.
  - Invites limited to 10 per minute, with a 60 s cooldown after a decline.
  - A setting for who can invite me: everyone or friends only.
- **Acceptance:**
  - [ ] After A blocks B, B's chat never reaches A, across matches and restarts (server test).
  - [ ] B's invites and requests fail silently.
  - [ ] An 11th invite within a minute is rejected.
  - [ ] The block survives a reinstall.
- **Depends on:** —

#### LR-40 — Nickname policy
Area: handles · **major** · M · code
- **Now:**
  - Nickname checks cover only length, charset and an optional tag (`shared/src/career.rs:12-34`).
  - A client-chosen tag is accepted (`career-store/migrations/postgres/002_player_handles.sql:6-10`).
  - There is no denylist, reserved-word list, confusable check, rename cooldown or name history (`career-store/src/career_store.rs:356-370`; `account-api/src/accounts.rs:197-214`).
  - Party presence allows U+202E (`shared/src/party.rs:133-145`).
  - Names are frozen into receipts (`shared/src/career.rs:87-102`).
- **Requirement:**
  - A shared `name_policy` used by registration, rename, the portal and presence. It:
    - normalizes homoglyphs and leet spelling
    - checks a versioned ru/en denylist file and reserved words (admin, mod, модератор, support, поддержка, omoba, official, staff, gm, dev, ekza)
    - enforces uniqueness on a UTS #39 confusable skeleton
  - Tags are assigned by the server.
  - Bidi characters are rejected, and unauthenticated presence shows "Guest".
  - Rename is limited to once per 7 days, with a history table.
  - After a `name_reset`, receipts show the current name.
- **Acceptance:**
  - [ ] Unit tests reject profanity and its variants, `Admin#0000`, `OMOBA#0001` and a Cyrillic homoglyph copy, while ordinary Cyrillic names pass.
  - [ ] A second rename within the cooldown gets a localized refusal.
  - [ ] U+202E in presence renders safely.
  - [ ] After a force-rename, the old name is gone from the portal and the game.
- **Depends on:** LR-15

#### LR-41 — Chat safety for RU
Area: chat · **major** · M · code+owner
- **Now:**
  - Chat is on in every market.
  - Validation is only structural (`shared/src/social.rs:99-114`; `server/src/social.rs:136-223`).
  - Match chat reaches opponents unfiltered.
  - Mute is local and resets every match.
  - The market Features have no chat flag (`shared/src/market.rs:37-50`).
  - The ОРИ question (149-FZ art. 10.1) and the age-category impact are undecided.
- **Requirement:**
  - Record the O-9 decision.
  - An RU `chat` market feature; when it is off, only reactions remain.
  - When chat is on:
    - a server-side mask using the LR-40 denylist
    - optional blocking of URLs and phone numbers in Match chat
    - a saved setting: All / Team only / Off
- **Acceptance:**
  - [ ] The decision is recorded in `docs/markets.md`.
  - [ ] RU build test: either chat is off and reactions work, or profanity arrives masked.
  - [ ] "Team only" persists across matches and restarts.
- **Depends on:** LR-40, LR-15, O-9

#### LR-42 — Account deletion and data erasure
Area: privacy · **major** · M · code+owner
- **Now:**
  - The API has no account-deletion route (`account-api/src/lib.rs:466-545`).
  - Receipts embed nickname and profile_id and are append-only (`career-store/migrations/postgres/001_career.sql:73-108`), and keys pin profiles through a foreign key.
  - The runbook forbids deleting match directories (`ops/vps/ru/README.md:57-58`).
  - RU Android creates accounts automatically (`docs/markets.md:18`).
- **Requirement:** "Удалить аккаунт" in the portal (with a fresh confirmation) and in the game. It:
  - revokes keys, sessions and the VK link
  - deletes friendships, blocks, settings, recovery codes, feedback text and analytics hashes
  - tombstones the profile as `Deleted#NNNN` with `deleted_at`, and refuses the deleted key afterwards
  - shows the tombstone name in history readers, and a job anonymizes the raw match JSON
  - writes an audit row

  The operator procedure and deadline for e-mail requests go into RUNBOOK and the policy.
- **Acceptance:**
  - [ ] Postgres test: no leftover rows remain for the profile.
  - [ ] Other players' history shows "Удалённый игрок", and their ratings are unchanged.
  - [ ] The deleted key is refused.
- **Depends on:** LR-04, O-15

#### LR-43 — Bot difficulty picker
Area: bot difficulty · **major** · M · code (+owner D1)
- **Now:**
  - Four difficulty levels are built and calibrated (`shared/src/bot_profile.rs`).
  - Plan step S4 (the picker) is unchecked (`docs/plans/bot-personas.md:984`).
  - Offline practice reads the level only from an env var and defaults to Elite (`client/src/net/offline.rs:34-44`).
  - Online is always Elite (`server/src/match_pool.rs:343`).
- **Requirement:**
  - A picker in offline hero select and standalone practice, applied to both teams, saved locally, defaulting to Regular.
  - The banner echoes the active level.
  - The owner decides online difficulty (O-6).
- **Acceptance:**
  - [ ] The picker offers four levels, defaults to Regular on first run, and the choice survives Leave, Rematch and restart.
  - [ ] An old host shows "Elite (server default)".
  - [ ] i18n parity and PickerLayout tests pass.
- **Depends on:** O-6

#### LR-44 — Guided first match (tutorial)
Area: onboarding · **major** · L · code
- **Now:**
  - No tutorial exists.
  - Onboarding is static help cards (`client/src/help_overlay.rs:79-110`).
  - PLAY sends Quick (`client/src/frontend/home.rs:1216-1226`).
  - Offline practice starts at level 6 (`common/src/offline.rs:20,143-147`).
- **Requirement:** A skippable offline guided match:
  - Steps complete only on authoritative actions: move, attack, last-hit, upgrade, cast, buy, recall, push and destroy the base.
  - Completion is saved locally and on the profile.
  - On first launch, Home highlights "Learn to play"; the tutorial can be reopened from Help.
- **Acceptance:**
  - [ ] Each step advances only on snapshot evidence.
  - [ ] RU, EN and zh texts are at parity.
  - [ ] It works with mouse, touch and controller.
  - [ ] A headless test drives it to completion.
- **Depends on:** LR-43, LR-08

#### LR-45 — Launch hero roster
Area: hero roster · **major** · M · code+owner
- **Now:**
  - 17 classes can be picked online (`shared/src/lib.rs:172-191`).
  - 12 of them are "playable prototypes" (`docs/progress/2026-09-29-remaining-roster.md:57-58,65-67`; `docs/features.md:314-322`).
  - There is no allowlist.
- **Requirement:**
  - The owner approves a launch roster (O-7).
  - Server Join enforces it for public and rated workers, and all classes stay available offline.
  - Each launch hero gets a playtest and a simulator win-rate band.
- **Acceptance:**
  - [ ] A server test refuses a non-roster class.
  - [ ] Hero select greys out non-roster classes online.
  - [ ] Evidence is documented for each launch class.
- **Depends on:** O-7

#### LR-46 — Graphics quality presets and published minimum specs
Area: performance · **major** · M · code
- **Now:**
  - `RenderSettings` holds only the FPS limit and the FPS counter (`client/src/render_settings.rs:18-41`).
  - Android turns shadows off (`client/src/world.rs:317-318`).
  - No minimum specs are published.
- **Requirement:**
  - Low/Medium/High presets controlling shadows, render scale, particles and post-effects, persisted.
  - Suggest Low on first run, or after a sustained p90 frame time over budget.
  - Developer sliders are hidden (done in LR-31).
  - Measured minimum and recommended specs per platform.
- **Acceptance:**
  - [ ] Each preset changes cost measurably.
  - [ ] The Low suggestion triggers.
  - [ ] Specs are published.
- **Depends on:** LR-25, LR-31

#### LR-47 — Leaderboards
Area: leaderboards · **major** · M · code
- **Now:**
  - There is no leaderboard route, page or screen (`account-api/src/lib.rs:523-544`; `market-web/src/lib/navigation.mjs:7,13`).
  - There is no rating index (`career-store/migrations/postgres/001_career.sql:2-12`).
- **Requirement:**
  - `GET /v1/leaderboard`: the top 100 by rating with at least N rated matches and public profiles only, plus the caller's own position.
  - A friends leaderboard.
  - A portal page and an in-game modal.
  - An index or a periodic snapshot.
- **Acceptance:**
  - [ ] Ordering is stable, with ties broken by rated_matches and then profile_id.
  - [ ] Non-public profiles are excluded.
  - [ ] The caller sees their own rank even outside the top 100.
  - [ ] Both surfaces render in RU.
- **Depends on:** LR-08

#### LR-48 — Level rewards and daily return hooks
Area: retention · **major** · L · owner+code
- **Now:**
  - XP is granted (`career-store/src/career_store.rs:671-682`), but level is display-only (`shared/src/career.rs:179-181`).
  - There are no unlocks, daily tasks or achievements, and every avatar is already available (`client/src/frontend/collection.rs:1-3`).
- **Requirement:**
  - The owner defines a reward track for levels 2-20 and daily hooks, such as a first-win bonus and challenges (O-14).
  - Rewards are granted inside the settle transaction, exactly once.
  - Post-match shows "Unlocked: …", and Home shows the next reward.
- **Acceptance:**
  - [ ] No double grant on retry (test).
  - [ ] The first-win bonus applies once per MSK day.
- **Depends on:** O-14

#### LR-49 — Desktop code signing and signed checksums
Area: packaging trust · **major** · M · owner (+S code)
- **Now:**
  - macOS builds are ad-hoc signed and not notarized (`scripts/release.py:757-758`).
  - Windows builds are unsigned, and the RU instructions tell players to bypass SmartScreen (`scripts/release.py:556-566,1376-1379`).
  - The exe has no icon or VERSIONINFO.
  - `SHA256SUMS` is unsigned (`scripts/release.py:271-275`).
- **Requirement:**
  - Windows: Authenticode, or distribution through VK Play.
  - macOS: notarization, or macOS excluded from the RU scope.
  - Embedded icon and VERSIONINFO.
  - A minisign signature on `SHA256SUMS`.
- **Acceptance:**
  - [ ] The Windows signature verifies (or the build ships via VK Play).
  - [ ] macOS is notarized, or its exclusion is recorded.
  - [ ] No bypass instructions remain.
  - [ ] Preflight verifies the signature.
- **Depends on:** LR-05, O-10

#### LR-50 — RuStore and VK Play listings
Area: store · **major** · M · owner
- **Now:**
  - Stores are an optional runbook step (`docs/markets.md:143-147`).
  - The media kit dates from 0.18 (`landing-v2/apps/omoba/docs/MEDIA-KIT.md:3-12`).
  - The RU native captures rendered black (`docs/progress/2026-10-24-market-profiles.md:56`).
- **Requirement:**
  - A RuStore listing in Russian with:
    - category, age sign and privacy URL
    - physical-device screenshots of the current RU build
    - icon and feature graphic
  - A VK Play page for PC, if the owner chooses it.
  - The median match length, taken from `portal.player_match_facts.duration_ms`.
- **Acceptance:**
  - [ ] Listings are live with copy the owner reviewed.
  - [ ] At least 4 Russian screenshots from a physical device.
  - [ ] The measured median length is stated.
- **Depends on:** LR-04, LR-07, LR-24, LR-25

#### LR-51 — DDoS posture
Area: infrastructure · **major** · M · owner+ops
- **Now:**
  - One IP hosts everything (`ops/vps/ru/README.md:3-6`; `docs/markets.md:123-127`).
  - The application-level limits (`server/src/public_transport.rs`, `ops/vps/ru/Caddyfile.ru`) do not stop volumetric floods.
  - There is no runbook.
- **Requirement:**
  - A provider with UDP anti-DDoS, or a protected proxy.
  - The web hosts separated from the game IP, or the accepted risk recorded.
  - An "under attack" runbook section.
- **Acceptance:**
  - [ ] The provider is recorded in `ops/vps/ru/README.md`.
  - [ ] `account` and `app` resolve separately from the game IP, or the risk is recorded.
  - [ ] RUNBOOK covers confirming an attack, the provider contact and switching to maintenance (LR-26).
- **Depends on:** LR-01, LR-26, O-16

#### LR-52 — Secret rotation and incident runbook
Area: secrets · **major** · M · ops+code
- **Now:**
  - The portal secret is the MAC key for sessions, recovery codes, device codes, tickets and VK subjects (`account-api/src/social.rs:249-374`; `account-api/src/devices.rs:306`).
  - The social secret is "never rotate".
  - Secrets have no versioning, and there is no incident section.
- **Requirement:**
  - Verify against the current and the previous secret, so rotation is possible.
  - A runbook for a leaked portal secret, a leaked social secret and a stolen database: revoke, rotate, notify.
- **Acceptance:**
  - [ ] v1 tokens and recovery codes still verify after rotating to v2 (test).
  - [ ] The incident section exists in `ops/vps/ru/README.md`.
- **Depends on:** LR-03

### M2 — Growth and retention

#### LR-53 — Ranked ladder presentation: tiers, placement, seasons, cross-play data
Area: rating model · **minor** · L · code+owner
- **Now:**
  - Rating is a bare i32 with K=32 and no tiers or placement (`shared/src/career.rs:9-10,153-182`; `docs/match-progression.md:143-146`).
  - There is one lifetime rating and no seasons (`001_career.sql:5`).
  - Matches record no input type or platform (`server/src/match_service.rs:15-25`).
- **Requirement:**
  - `tier(rating, rated_matches)` with owner-set thresholds.
  - A provisional K and "Calibrating i/N".
  - Tier badges on every profile surface, plus a promotion/demotion banner.
  - A seasons table and a close-season command (soft reset plus archive).
  - The input type stored per participant, and a cross-play policy (O-18).
- **Acceptance:**
  - [ ] Unit tests cover tier boundaries.
  - [ ] Badges show in RU and EN.
  - [ ] Closing a season archives and soft-resets in one transaction.
  - [ ] A win-rate-by-input query exists.
- **Depends on:** LR-08, LR-47, O-5

#### LR-54 — History and statistics polish
Area: history · **minor** · M · code
- **Now:**
  - Portal filters accept only 4 of 17 classes and default to rated, so players who only played bots see empty stats (`account-api/src/read.rs:71-88`; `market-web/src/components/portal.tsx:70-74`).
  - Facts and summaries have no mode or XP (`account-api/migrations/001_portal.sql:67-87`; `shared/src/career.rs:135-150`).
  - Matches aborted before start count as played (`server/src/career_runtime.rs:787-806,837-844`).
  - Interrupted matches give no XP (`career-store/src/career_store.rs:586-594`).
- **Requirement:**
  - The class list comes from `HeroClass::ALL`, and stats default to all matches.
  - A mode column with a backfill, and a mode filter.
  - History rows show a mode badge and "+N XP".
  - A match aborted before combat settles as cancelled and does not count as played.
  - Interrupted matches give pro-rated XP.
- **Acceptance:**
  - [ ] The `warden` filter returns data.
  - [ ] A player with only bot games sees stats.
  - [ ] An abort during loading leaves `matches_played` unchanged.

  The default-stats fix is S and worth doing early.
- **Depends on:** LR-08

#### LR-55 — Party robustness and lobby party chat
Area: party · **minor** · M · code
- **Now:**
  - Parties live in memory (`docs/party.md:25-31,69-73`).
  - There is no gather timeout and no "waiting for X" (`server/src/match_service.rs:380-405`).
  - Ranked has no stack-size rule.
  - Party errors are only printed to the server log, and `PartyView` has no error field (`server/src/party.rs:556-558`; `shared/src/party.rs:107-117`).
  - There is no party chat (`client/src/social.rs:46-48`).
- **Requirement:**
  - A gather timeout that names the missing member and lets the party launch without them.
  - The O-4 stack rule.
  - Localized party error codes.
  - Parties re-form after a restart.
  - Party-only lobby chat with filtering and blocks.
- **Acceptance:**
  - [ ] An idle member is named within 60 s.
  - [ ] A test covers the stack rule.
  - [ ] "party is full" is localized.
  - [ ] Non-members never receive party chat.
- **Depends on:** LR-08, LR-39, LR-41

#### LR-56 — Custom/private matches
Area: modes · **minor** · M · owner
- **Now:** No custom path exists, and a party always plays on one team (`shared/src/match_service.rs:5-12`; `server/src/match_service.rs:426-479`).
- **Requirement:** A private lobby by code or invite, with team seats and bot fill, unrated.
- **Acceptance:**
  - [ ] A Custom allocation path with seat validation.
  - [ ] The owner decides when it ships (O-17).
- **Depends on:** LR-08

#### LR-57 — Server directory, global hostname and scaling path
Area: directory/scaling · **minor** · L · code+ops
- **Now:**
  - The directory is only a concept (`docs/plans/game-directory.md:3-5`).
  - The global build hard-codes the IP `77.246.105.57:4000` (`client/src/session_config.rs:31-36`).
  - The global unit has `Requires=docker.service` (`ops/vps/omoba-lobby.service:3-5,22`).
  - One lobby runs per host (`server/src/match_pool.rs:34-47`).
- **Requirement:**
  - A hostname as the global default.
  - A signed directory document (min release, update URL, maintenance) with a fallback.
  - The Docker requirement dropped.
  - A documented vertical-scaling procedure.
  - Plan steps D0/D1 scheduled before the global launch.
- **Acceptance:**
  - [ ] Resolver and bad-document tests.
  - [ ] Restarting Docker keeps the lobby running.
  - [ ] RUNBOOK has a resize section.
- **Depends on:** LR-09, LR-14

#### LR-58 — Bot quality and fairness
Area: bots · **minor** · L · code
- **Now:**
  - Green wins 64.9% of games, with the cause unknown, and a lone party always plays Green (`docs/progress/2026-10-14-bot-personas.md:90`; `server/src/match_service.rs:423-424,450-476`).
  - Bots use only 5 classes and ignore draft roles (`common/src/bots.rs:29-37,1394-1412`).
  - Plan steps S5-S13 (personas and AI labels) are unchecked.
- **Requirement:**
  - Find and remove the side bias, and randomize the side.
  - Bots pick from the launch roster to complement human roles.
  - S5 AI labels first, then personas.
- **Acceptance:**
  - [ ] The Wilson interval for Green's win share contains 0.5 over 200 or more rounds.
  - [ ] A test covers random sides.
  - [ ] When a human picks Mid, no bot on that team is Mid.
- **Depends on:** LR-45

#### LR-59 — Onboarding and controls polish
Area: onboarding · **minor** · M · code
- **Now:**
  - There is no first-run nickname step (`client/src/career.rs:93`).
  - There are no starter or complexity tags.
  - Offline practice always starts at level 6 (`common/src/offline.rs:20`), and its plan is stale (`docs/plans/offline-practice-parity.md`).
  - Help omits recall, dash and haste (`client/i18n/en/help.json:4-19`).
  - Keys cannot be rebound (`client/src/input_bindings.rs:8-14`).
  - There is no fullscreen mode, saved window geometry or window icon (`client/src/platform.rs:37-54`).
- **Requirement:** Add each of the following:
  - a nickname step
  - complexity tags and starter heroes
  - an offline "Normal match" starting at level 1
  - full key help and rebinding
  - display mode, saved geometry and an icon
- **Acceptance:** Each item works as described, and i18n parity passes.
- **Depends on:** LR-31

#### LR-60 — Desktop installers and macOS coverage
Area: packaging · **minor** · M · code
- **Now:** macOS is arm64-only, and Windows ships as a bare zip (`scripts/release.py:14-15,242-243`).
- **Requirement:**
  - A Windows installer with shortcuts, an uninstaller and in-place upgrades.
  - An x86_64 or universal macOS build, or "Apple silicon only" stated on the download page.
- **Acceptance:**
  - [ ] Both artifacts are in the catalog with checksums.
  - [ ] An upgrade installs in place.
- **Depends on:** LR-49

#### LR-61 — Russian UI layout capture pass
Area: localization QA · **minor** · M · code
- **Now:** The UI audit captures English only (`scripts/ui_screen_map.json:963-969`); Russian has only spot checks.
- **Requirement:** RU captures of every screen on desktop and phone, with an overflow check.
- **Acceptance:**
  - [ ] `make ui-audit LANG=ru` runs.
  - [ ] Key screens fit at 852x393 and 1280x720.
- **Depends on:** LR-31

#### LR-62 — Privacy settings enforcement
Area: privacy · **minor** · S · code
- **Now:**
  - The lobby list ignores the discoverable setting (`server/src/party.rs:468-500`).
  - Any friendship row, including an outgoing pending request, unlocks private stats (`account-api/src/accounts.rs:38-59`; `career-store/src/career_store.rs:372-380`).
- **Requirement:**
  - A contact is an accepted friend, or the recipient of an incoming request.
  - A "show me in lobby" setting and an appear-offline option.
- **Acceptance:**
  - [ ] A requester whose request was not accepted gets 403.
  - [ ] A hidden player is absent from the lobby list but can still be invited.
- **Depends on:** —

#### LR-63 — Web and database hardening
Area: web/db · **minor** · S · code+ops
- **Now:**
  - Neither RU host sends CSP or HSTS (`market-web/next.config.ts:18-27`; `ops/vps/ru/Caddyfile.ru`).
  - `game_role` has UPDATE and DELETE on career tables (`account-api/ops/grants.sql:30-36`).
- **Requirement:**
  - A strict CSP and HSTS with max-age ≥ 15552000.
  - Unneeded grants revoked.
- **Acceptance:**
  - [ ] `curl -I` shows both headers.
  - [ ] Playwright sees no CSP violations.
  - [ ] A grants test shows `game_role` cannot delete profiles.
- **Depends on:** LR-02

#### LR-64 — Transport and device-key hardening
Area: transport/client key · **minor** · L · code
- **Now:**
  - The server is not authenticated to the client, and UDP is not encrypted (`shared/src/public_transport.rs:1-3`).
  - The device seed is plaintext (`client/src/career_identity.rs:28-32,57-77`).
- **Requirement:**
  - The client verifies snapshots and allocations against a pinned key.
  - The seed moves to the OS keystore, with a migration that keeps the public key.
- **Acceptance:**
  - [ ] A spoofed snapshot is ignored.
  - [ ] No plaintext seed remains after migration, and the profile is unchanged.
- **Depends on:** —

#### LR-65 — Web analytics in Russia and cookie notice
Area: analytics · **minor** · S · code
- **Now:** The landing loads Supabase and Vercel analytics (`landing-v2/apps/omoba/src/app/layout.tsx:2,5,143-144`); there is no Metrica and no cookie notice.
- **Requirement:** Yandex Metrica with a cookie notice and opt-out on the RU landing and portal, and no foreign analytics on RU pages.
- **Acceptance:**
  - [ ] The counters receive data.
  - [ ] Declining disables tracking.
- **Depends on:** LR-22, LR-04

#### LR-66 — iOS for RU
Area: distribution · **minor** · S · owner
- **Now:** The RU market has no iOS build (`docs/markets.md:21,149`; `scripts/markets.py:66-67`).
- **Requirement:** Decide between an RU App Store record, using the global build, or stating that iOS is not available.
- **Acceptance:**
  - [ ] The decision is recorded in `docs/markets.md` and on the RU landing.
- **Depends on:** —

---

## 4. Owner and ops checklist

**Legal**
- [ ] Choose the operator entity (ИП, ООО or self-employed) — LR-05
- [ ] File the RKN operator notice and record the cross-border position — LR-05
- [ ] Counsel drafts and approves the privacy policy, user agreement and consent in Russian — LR-04
- [ ] Decide whether chat is allowed in RU (ОРИ, 149-FZ) — LR-41, O-9
- [ ] Choose the age category — LR-07
- [ ] Write the manual data-deletion procedure and its deadline — LR-04, LR-42

**Accounts and keys**
- [ ] Create the VK ID app (redirect, scopes, OK and Mail.ru, legal URLs) — LR-19
- [ ] Decide custody and escrow for the Android release keystore — LR-24, LR-03
- [ ] Open RuStore and, optionally, VK Play developer accounts — LR-50
- [ ] Decide on a Windows code-signing certificate or VK Play, and on Apple Developer ID or dropping macOS — LR-49

**Hosting and ops**
- [ ] Buy the RU VPS, ideally from a provider with UDP anti-DDoS, and size it from LR-09. Set DNS A records — LR-01, LR-51
- [ ] Set up backup storage in Russia and name the secrets escrow holder — LR-03
- [ ] Set up the download host in Russia — LR-23
- [ ] Set up an external probe host or uptime service and a Telegram alert bot — LR-17
- [ ] Record the egress allowance for the expected number of concurrent players — LR-28

**Community and launch**
- [ ] Create the VK group, the RU Telegram channel and chat, a support e-mail, and name at least one moderator — LR-21
- [ ] Write the launch plan, including seeding sessions to fill human matches — LR-21
- [ ] Approve the RU landing copy and the release status label — LR-22, LR-23
- [ ] Review feedback, reports and the daily digest weekly — LR-20, LR-30, LR-38

---

## 5. Critical path and effort

M0 in dependency order, with sizes (S ≈ 0.5, M ≈ 1.5 engineer-weeks):

1. LR-05 Operator and RKN notice (S, owner) → LR-04 Legal documents (M, owner). Start now; this track's calendar time does not depend on engineering.
2. LR-01 RU stack provisioned (M, ops)
3. LR-02 Portal ↔ Account API topology (S) · LR-03 Backups and escrow (S) · LR-23 RU downloads (S)
4. LR-17 Monitoring, alerts, retention (M)
5. LR-11 Match lifecycle cap (S) → LR-12 Watchdog (S) · LR-13 Re-queue state (S)
6. LR-10 Bot CPU budget (M)
7. LR-09 Capacity measured and sized (M, after LR-10 and LR-11)
8. LR-08 Ranked/Casual/Bots queues (M, upper end)
9. LR-16 Admission and abuse limits (M) → LR-15 Moderation minimum (M)
10. LR-14 Client update path (M)
11. LR-06 Consent and legal links (M) · LR-07 Age marking (S, owner)
12. LR-19 VK ID app (S, owner) → LR-18 Protect progress (M)
13. LR-20 Feedback (M) · LR-21 Community and support (S, owner)
14. LR-22 RU web entry point (M)
15. LR-24 Android release build (S) → LR-25 Android device validation (M)
16. Final step: run the RU release checklist from LR-01 on every platform.

**Effort.** M0 engineering is about 24 engineer-weeks (range 20-30); owner items are excluded.

| Track | Specs | Engineer-weeks |
|---|---|---|
| A — infra/ops | LR-01, 02, 03, 23, 17, 09 | ≈ 6 |
| B — server/matchmaking | LR-11, 12, 13, 10, 08, 16, 15 | ≈ 8 |
| C — client/web | LR-14, 06, 18, 20, 22, 24, 25 | ≈ 8.5 |
| D — owner | LR-05, 04, 07, 19, 21 | runs in parallel |

**Parallelism.** The three engineering tracks can start together. With three engineers, M0 takes about 9-10 calendar weeks including integration and the release-checklist run; with two, about 12-13 weeks. The cross-track handoffs are:
- LR-09 waits for LR-10 and LR-11.
- LR-18 waits for LR-02 and LR-19.
- LR-06, LR-19 and LR-22 need the URLs from LR-04, but placeholders let the code land first.

M1 is about 50 engineer-weeks (six L specs plus about 19 M specs), owner items excluded. Do these first after launch:
- LR-26/LR-27 so deploys do not kill matches
- LR-29/LR-30 so crashes and metrics are visible
- LR-31 so players see no English or raw codes
- LR-32/LR-33/LR-35 once real Ranked traffic appears

---

## 6. Open decisions for the owner

| ID | Decision | Changes |
|---|---|---|
| O-1 | Is Ranked available at launch or only later? At low population: a minimum-population gate, ranked hours, a smaller ranked format, or wider rating windows? (Bots are never rated.) | LR-08, LR-35 |
| O-2 | Should bots fill Casual PvP, and after how long (30 s today)? Should Casual merge cohorts or wait longer while other humans are online? | LR-08, LR-36 |
| O-3 | Ranked entry gate: what career level or match count, and must the player link VK ID in RU? | LR-37 |
| O-4 | Ranked parties: solo/duo only, or full premade against premade? | LR-55 |
| O-5 | Seasons: when, how long, soft-reset formula, rewards? Tier thresholds and placement count? | LR-53 |
| O-6 | Online bot difficulty: keep Elite for all, or easier for newcomers' first matches? Should Server bot match offer a level picker? | LR-43 |
| O-7 | Launch hero roster: which of the 17 classes are allowed in public and ranked play? | LR-45, LR-58 |
| O-8 | Match length target, max duration, the minute from which surrender is allowed, map tier preset for casual and bot play | LR-11, LR-34 |
| O-9 | RU chat: off (reactions only), filtered, or fully on? Interacts with ОРИ status and the age category | LR-41, LR-07 |
| O-10 | Distribution: RuStore, VK Play or direct download only? Desktop signing? Is macOS in the RU launch? iOS for RU? | LR-49, LR-50, LR-60, LR-66 |
| O-11 | Android: Vulkan-only with a published minimum device, or invest in a GLES fallback? | LR-25 |
| O-12 | Update policy: move balance values into server data, or keep releasing client and server together with a forced update? | LR-14 |
| O-13 | Leaver/AFK thresholds, cooldown ladder, remake window | LR-33 |
| O-14 | Contents of the reward track and the daily hooks | LR-48 |
| O-15 | Account deletion: self-service at launch, or a manual e-mail procedure first? Does the target store require in-app deletion? | LR-42, LR-04 |
| O-16 | Hosting: one VPS vs separate web and game IPs, provider with UDP anti-DDoS, host city, budget for more capacity | LR-01, LR-09, LR-51 |
| O-17 | Are custom/private matches in the first public release? | LR-56 |
| O-18 | Ranked cross-play between touch and mouse players | LR-53 |
| O-19 | Consent model for attaching logs and crash data to feedback and crash reports | LR-20, LR-29 |
| O-20 | When does the global market launch? This decides when the directory and global hostname work (D0/D1) is needed | LR-57 |
| O-21 | Production app identity for the one app per platform. Default: a new id (for example `io.omoba.game`) chosen before the first public listing, before any IAP product is created and before the first public RU APK. Changing it later means a new app, lost reviews and new product ids. Android: own key uploaded to Play App Signing. | LR-68, LR-69, LR-24, LR-50 |
| O-22 | Who owns the international contour (omoba.io, `account.omoba.io`) and the Apple developer account: the Russian ИП or ООО, or a non-Russian entity? This decides: 149-FZ art. 8 part 10 (Russian-citizen or Russian-entity owners); whether our own first-party sign-in qualifies (it needs an owner who is an RF citizen without other citizenship, or a Russian entity); 152-FZ art. 18 part 5; the public EU DSA trader details (address, phone, e-mail), or else the EU storefronts are excluded; and whether a Russian-resident operator may accept USDC on `app.omoba.io` under the digital-currency rules in force since 2026-09-01 (unverified; counsel). | LR-68, LR-69, LR-05 |
| O-23 | Sign in with Apple at R2b: gated by Russian signals (default), not offered at all, or only after O-22 moves the contour to a foreign entity? | LR-68 |
| O-24 | Apple payouts and tax: can the account holder receive payouts (bank country, sanctions)? Currency control, tax-agent VAT on Apple's commission, the 54-FZ position, Small Business Program eligibility. This gates product creation in App Store Connect. | LR-69 |
| O-25 | Google Play: publish the unified app (default) with the RU YooKassa link under Play's Russia exemption and no purchase entry for global, or skip Play? Requires in-app plus web deletion and Data safety. The RU region is open to non-residents while the exemption covers users in Russia; counsel to confirm. | LR-68 |
| O-26 | RuStore: may the RU build link to the YooKassa page, or must it use RuStore payments? A written answer from RuStore is needed before the RuStore listing. | LR-50, LR-67, LR-68 |
| O-27 | Solana: keep 0 tier points (default; global unchanged), or give a Solana grant tier points (for example 500 = Azure), which also requires lifting the `already_supported` stacking block? | LR-67, LR-69 |
| O-28 | Approve Apple's App Store Server Library as a production dependency of `billing/apple-verifier/`. No Apple sale can be verified without it. | LR-69 |
| O-29 | Confirm the price ladder ($0.99 … $99.99) and the 7 product names. Optionally set manual prices in KZ, TR, AM, GE, KG and UZ at or above ruble parity (default: Apple's equalization). | LR-69 |
| O-30 | Russian citizens' data on the global contour: where is the global host (`77.246.105.57`), and who operates it? Do existing global profiles of RF citizens need moving or erasure? Is a self-declaration on the region card enough for citizens abroad who pick International, and with what wording? Is transient receipt of RU-account Apple payloads abroad acceptable? The RKN notice (LR-05) lists every database location and any cross-border transfer. | LR-68, LR-69, LR-05 |
| O-31 | The international contour: the global lobby DNS name (D0); a deletion intake for international accounts before R2a (a minimal `account.omoba.io` with only deletion and releases, or a signed lobby action); the full Account API and portal accounts for R2b (host, data location, operator). | LR-68, LR-69, O-20 |
| O-32 | Is the RU region open to anyone (default yes, for example Russian-speaking players in the CIS), with cards worded by servers and language? | LR-68 |

---

## 7. Default decisions for the M0 implementation (2026-10-24)

The owner asked to bring the RU launch to maximum readiness before the RU host
exists. The M0 code specs are implemented against these defaults. Each is a
single, documented setting the owner can change later.

| ID | Default | Where it lives |
| --- | --- | --- |
| O-1 | **Ranked at launch**: humans-only 5v5, rated, never bot-filled. The searching screen shows how many players search in the queue; after 5 minutes it offers switching to Casual without losing the place in Ranked. | LR-08, `docs/match-progression.md` |
| O-2 | **Casual PvP**: unrated, awards XP, groups every human in the queue regardless of rating or newcomer cohort, then bot-fills after 30 s. **vs Bots** stays a separate choice. | LR-08 |
| O-3 | No ranked entry gate beyond the existing newcomer and eligibility rules (LR-37 later). | LR-08 |
| O-6, O-7 | Online bot difficulty and the hero roster stay as they are (LR-43, LR-45 later). | — |
| O-8 | Hard match cap **45 minutes**. A capped or abandoned match settles as **Interrupted** (unrated, never counted twice), reusing the existing semantics. | LR-11, LR-12 |
| O-9 | **RU chat off** at launch: a server-wide chat switch (`OMOBA_CHAT=off` in the RU lobby unit); reactions/emotes stay. Turning it on later needs the owner's legal check (ОРИ, 149-FZ). | LR-15, LR-41 |
| O-10 | RU distribution: direct downloads from the RU host (IPFS as mirror) for Windows, Linux, macOS and Android. RuStore/VK Play and iOS later. | LR-23, LR-50, LR-66 |
| O-11 | Android: Vulkan only, with a localized "device not supported" screen and published minimums; no GLES fallback. | LR-25 |
| O-12 | Update policy: client and server ship together; an incompatible client shows **Update required** with a download button; a newer compatible release shows a dismissible banner. Source: the market's release catalog (`GET /v1/releases`). | LR-14 |
| O-15 | Account deletion: a manual e-mail procedure at launch, described in the privacy policy (self-service later, LR-42). | LR-04 |
| O-16 | One RU VPS runs PostgreSQL, the lobby and workers, the Account API, the portal and the download host behind Caddy. | LR-01, LR-23 |
| O-19 | Feedback and crash reports attach logs only when the player ticks a box (off by default). | LR-20 |
| — | **RU landing** = the RU portal's guest home at `omoba.ru` (hosted in Russia, no foreign analytics); the omoba.io landing is not changed for M0. | LR-22 |
| — | **Legal texts**: Russian drafts of the privacy policy and user agreement ship with a visible "проект" banner and version `0-draft`; the owner replaces them with the counsel-approved texts (LR-04) before publication. | LR-04, LR-06 |
| — | **Age category**: plumbing with the RU value `12+` as a placeholder the owner confirms with counsel before publication (with chat off). | LR-07 |
| — | **Release label**: «Открытая бета». | LR-23 |
| — | **Owner notifications** (feedback, alerts): one Telegram bot configured on the RU host (`OMOBA_TELEGRAM_BOT_TOKEN`, `OMOBA_TELEGRAM_CHAT_ID`); without it, notifications go to the journal only. | LR-17, LR-20 |
| — | **Schema numbers** reserved for M0: career core migration 004 = admission/new-profile limits (LR-16), 005 = sanctions (LR-15); portal migration 008 = feedback (LR-20). | LR-15, LR-16, LR-20 |
| — | **RU monetization (owner, 2026-10-24)**: the RU market sells supporter auras through YooKassa (LR-67). This replaces the earlier "no payments in RU" assumption: where specs say the RU build sells nothing, read "sells only YooKassa supporter auras; no Apple, Solana, crypto or Ekza". | LR-67 |
| — | **RU surfaces**: no donate-with-crypto, wallet, Solana or Ekza content; the RU Supporter page (YooKassa) and `/supporters` are allowed and linked from the portal navigation. | LR-22, LR-67 |
| — | **Legal drafts** (LR-06) also include a draft **public offer** (оферта) for supporter cosmetics at `/offer`: what is sold (a 30-day cosmetic aura, no gameplay advantage), prices, payment through YooKassa, refunds, and placeholders for the seller's requisites (ФИО, ИНН, ОГРНИП, address, e-mail, phone). Prices and the offer are readable without sign-in. | LR-06, LR-67 |
| — | **Reserved numbers**: portal migration 009 = YooKassa supporter (LR-67); game protocol 12 = tier aura styles (LR-67). | LR-67 |
| — | **RU domain (owner, 2026-10-24)**: the RU market lives on **omoba.ru** (registered at REG.RU, DNS there): `https://omoba.ru` = RU portal and landing (VK ID callback `https://omoba.ru/api/auth/vk/callback`, `/privacy`, `/terms`, `/offer`, `/supporter`, `/download`), `https://account.omoba.ru` = Account API (YooKassa webhook `https://account.omoba.ru/v1/supporter/yookassa/notifications`), `lobby.omoba.ru:4000` = lobby and match workers. They replaced the earlier `*.ru.omoba.io` names everywhere on 2026-10-24 (`scripts/markets.py` zone `omoba.ru`; clients, release tooling and the dev host tool treat `omoba.ru` as a production zone). New code and docs take names from `scripts/markets.py` / `shared/src/market.rs`, never literals. The global ecosystem (`omoba.io`, Ekza) is untouched; Ekza comes to RU later feature by feature (`--feature ekza=on`). | all RU specs |
| — | **One app per platform (owner, 2026-10-24)**: production clients carry `global` and `ru`. On first launch the player confirms one region card with its legal acceptance; the suggestion comes from the device region or time zone, with no network. Switching takes effect after a cold start, with a blocking restart screen on phones. Each region keeps its own profile and folder. Servers, the Account API and the portal stay one deployment per region. | LR-68, ADR-025 |
| — | **Build channel**: `OMOBA_CHANNEL = app_store \| google_play \| rustore \| direct`, set by the packager. It decides the purchase surface and the update link. | LR-68 |
| — | **Purchase surface**: `app_store` = StoreKit (RU from R2a, global from R2b). `google_play` = link to `omoba.ru/supporter` for `ru` (Play's Russia exemption), none for `global`. `direct` = the region portal's `/supporter`. `rustore` = none until O-26. | LR-67, LR-68, LR-69 |
| — | **iOS tier products**: seven Non-Renewing Subscriptions, 30 days from purchase. US base $0.99 / $2.49 / $4.99 / $9.99 / $24.99 / $49.99 / $99.99 (Rose … Prism III); other storefronts equalized. Ids are `<contour bundle>.aura.<tier>.30d`. The monthly product is retired; it was never sold. | LR-69 |
| — | **Tier points**: the tier is the sum over covering grants. YooKassa 1 ₽ = 1 point; Apple products give their threshold; Solana gives 0 until O-27. The base styles need any active grant. Points are never displayed; stacking is disclosed in plain words. | LR-67, LR-69 |
| — | **Apple verification**: done by the account's region. First purchases come from device delivery; restore uses the server's transaction history; refunds come from a daily refund-history poll. There is no production notification URL until R2b and counsel. A foreign region's transactions are dropped in memory. | LR-69 |
| — | **RU sign-in**: the same labels on all channels (VK ID, OK, Mail.ru named plainly), plus a recovery code and device pairing as equal options. iOS uses an ephemeral `ASWebAuthenticationSession` with the `omoba-auth` callback. | LR-68 |
| — | **Unlink**: Profile → Devices lists linked sign-ins, each with Unlink (VK, OK, Mail.ru, Apple). | LR-68 |
| — | **Sign in with Apple**: international only (R2b), no scopes. It is hidden and refused for the RUS storefront, RU device region or RU IP (local GeoIP); only a reason code is logged. | LR-68 |
| O-15 (store channels) | **In-app deletion request** in Profile → Settings on every store channel, plus a public `/delete` page on each portal (Play Data safety). The key is refused at once; erasure follows within 30 days under LR-42. `direct` builds keep the e-mail procedure until LR-42 ships. | LR-69, LR-42 |
| — | **iOS identity**: the device key is also kept in the Keychain (this device only, no iCloud sync), so a reinstall keeps the profile. | LR-68 |
| — | **App identity**: a new production identity (for example `io.omoba.game`, O-21) before the first public listing and before any IAP product exists. The same id is used across Play, RuStore and direct downloads, signed with one owner key enrolled in Play App Signing. | LR-68, LR-24 |
| — | **Store-mode portal pages** (`?app=<channel>` in the URL, never in the session): no Supporter navigation, no `/supporter` or `/offer` links, no payment-method text. | LR-67, LR-68 |
| — | **Region for players**: RU is open to anyone. Cards describe servers and language, not residence (O-32). | LR-68 |
| — | **Reserved numbers**: portal migrations in merge order, currently 010 = LR-69 Apple tier products, 011 = Sign in with Apple. No game protocol bump for regions: `ReleaseContract.region` is an optional field; friend-code prefixes are client-side. | LR-68, LR-69 |

### Rules for M0 items so they survive LR-68 (2026-10-25)

LR-68 turns the compile-time market into a runtime region in one app per platform. M0 work follows these rules so it does not have to be rewritten:

Rules for every item:
- Read region values only through one accessor. Until R0 lands, that is one function in your module over `shared::market::{MARKET, FEATURES}`; R0 swaps its body for `shared::region::active()`.
- Add no new `MARKET == Ru` branches and no host or URL literals.
- Do not add `OnceLock::get_or_init` region state. Assume one binary may later run either region.

- **LR-06 (consent):**
  - Legal URLs (privacy, terms, offer, source, support, delete) go in the market/region table, not in screen code.
  - Store acceptance in the region folder as `{region, doc_version, accepted_at}`.
  - Gate traffic with one run condition, `bound_and_accepted`.
  - Build the screen as a card that can later carry the region choice. Region and legal become one Confirm.
- **LR-14 (update path):**
  - The catalog URL comes from the region's `account_api`.
  - The compatibility reply stays `ReleaseContract`, with no protocol bump for new optional fields.
  - The catalog carries `min_release`, `update_url` and `channel` (`direct` now).
  - The button target is chosen per (channel, platform). Never show a web download in an iOS build.
- **LR-18 (protect progress):**
  - In-game recovery-code entry is a first-class sign-in option. Later it sits next to the website button with equal prominence (Guideline 4.8).
  - Keep the route in the common Account API code, not behind `vk_sign_in`.
  - Prompt copy comes from the region's sign-in methods.
- **LR-20 (feedback):**
  - `POST {region.account_api}/v1/feedback` with `region` and `channel`, and no global fallback.
  - Record what is sent (free text, version, logs on opt-in) for the App Privacy and Data safety answers.
- **LR-22 (RU web entry):**
  - Use only `omoba.ru` names from `scripts/markets.py`.
  - Put one navigation switch on Supporter links, `/supporter` and `/offer`.
  - Keep Terms and Privacy text free of payment-method details, or split those into `/offer`, so store-mode variants are cheap.
  - Add a public `/delete` page.
- **LR-23 (RU downloads):**
  - `releases.json` entries carry `region` and `channel` (`direct`) next to the platform.
  - URLs are on `omoba.ru`; IPFS stays a mirror.
- **Packaging:**
  - Do not publish a public RU APK under `space.ekza.omoba.ru` before O-21.
  - Create the release keystore for LR-24 so that the same key can be enrolled in Play App Signing.
  - Keep `versionCode` monotonic across channels.
