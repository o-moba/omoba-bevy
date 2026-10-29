# RU registration and game sign-in

The RU portal and game use one regional account database. A website signup can
create a profile before a game device exists; an existing game's progress is
linked only after that game signs an explicit confirmation. There is no automatic
profile merge and no discovery by submitting credentials to both regional backends.

## Website first

The RU portal also embeds VK ID OneTap for the owner's app `54863720`.
Its registered redirect is the exact portal root, `https://omoba.ru`; ordinary
redirect buttons retain `/api/auth/vk/callback`. Both are bound to the configured
portal origin, with no other path/host accepted. The SDK receives only a public
PKCE challenge; the BFF retains its verifier and exchanges the callback through
this Account API. No provider token or user-id claim supplied by browser code can
establish an OMOBA session. See omoba-web `docs/vk-one-tap.md` for the CSRF-bound
prepare/complete endpoints and production activation gates.

1. Open the RU portal and choose VK ID, OK or Mail.ru. All three are entry methods
   of the same VK ID application; see the [official provider enum](https://github.com/VKCOM/vkid-web-sdk/blob/master/src/widgets/oauthList/types.ts).
2. The BFF starts authorization with PKCE and a state cookie. The API exchanges
   the code, verifies returned state and reads the provider user identity itself.
3. Ordinary website login requests `register:true`. A new verified subject gets
   exactly one profile with database-owned default rating/progression and a browser
   session. Existing subjects return their existing profile. Concurrent callbacks
   cannot allocate two profiles for that subject.
4. Download/open the matching game. A locked RU package already selects Russia;
   a unified package asks the player to choose Russia. Open Profile → Sign in with
   VK ID and use the same provider account. The website reviews the profile,
   device label and key fingerprint before approval.
5. Confirm in the game and cold restart to activate the enrolled device key. The
   game authenticates that key against the RU lobby and receives the same profile.

A new website account has no game device yet. It cannot unlink its only sign-in
until it has an active game key or a usable recovery code.

## Game first / existing progress

The game opens a short-lived `/connect?code=…` bridge directly in the system
browser; visiting the landing or typing the site's address is unnecessary. A new
social identity in this flow remains unlinked (`register` absent/false). After the
browser reviews the device and continues, the game must sign the link with its
current active key. This preserves that profile's progress. A known linked account
instead offers explicit sign-in to that account; it is not merged with the current
profile. Completion stages a candidate key and activates it only after restart.

A website signup made before linking existing progress can create a separate blank
account. The login page and landing therefore direct existing players to link from
the game first. Do not guess that two profiles belong to one person or merge them.

## Boundaries and recovery

- RU endpoints, identities and state remain separate from International. API device
  replies identify `account_region`; the native transport refuses foreign replies.
- The game uses its own Ed25519 device key and scoped game/session proofs. Provider
  tokens are exchanged server-side and do not enter game packets or browser storage.
- Choosing another social identity replaces/clears the previous browser identity.
  Device approval includes the reviewed profile id, so switching tabs/accounts
  cannot approve a different account from the one the page reviewed.
- A new link rechecks the active game key after locking the profile; revocation
  cannot race a stale key into a new recovery/sign-in method.
- Device-status polling has a separate IP quota from create/complete/recover.
  Native temporary failures retry the same unexpired proof with bounded backoff
  (3/6/12/24/24 seconds) and an explicit check action. Completion must be consumed
  for the expected valid profile before a candidate key can be staged.
- Invalid state, replay, cancellation, expiry and provider outage remain failures;
  retry preserves the intended `/connect` destination. Existing keys/progress stay
  intact while authorization is pending.

## Schema and deployment boundary

Portal migration `010_social_registration.sql` adds only the bounded
`portal.register_social_profile` function. It locks an existing verified identity,
allocates its own id and inserts only the fixed nickname; database defaults own
stats. PUBLIC cannot execute it. The portal runtime receives EXECUTE, not general
profile INSERT/stat-update grants. Runtime requires versions `[1,2,3,4,5,6,7,9,10]`.
Apply the release's grants after migration, through the owner-approved rollout.
The next planned Apple tier/sign-in migrations must use 011/012 in merge order.
No gameplay wire change: protocol remains 12 in this dev candidate.

The current dev activation helper still pins historical portal versions through 7;
its compatibility update is a separate deployment-tooling gate, not a reason to
skip schema validation or widen permissions. No production or CI configuration is
changed by this registration delivery.

## Verification and remaining live gates

The local rehearsal uses a real RU portal production build, real Account API,
PostgreSQL18, Chrome at ru 852×393 and three independent loopback provider subjects.
The provider double checks PKCE, state, device and one-use codes. Exported synthetic
native keys then run through a real public lobby, allocation, signed worker auth,
draft readiness, running bot match, movement and reconnect. Secrets/keys are private
fixture inputs, not public evidence. This proves our integration, not real provider
accounts, distributed packages or physical devices.

Read-only public checks on 2026-10-28 found `omoba.ru` at 95.163.244.138 with `/login`
404, no `account.omoba.ru`/`lobby.omoba.ru` DNS, and only the game service active on
the new 212.113.117.105 VPS. Its earlier provisioned main 0.44/protocol 11 is not the
candidate's protocol 12. Public RU registration is therefore not live-ready.

Before public acquisition: owner-approved main/candidate rollout; RU DNS/TLS,
portal/API and schema 10/grants; matching public app id and registered VK callback;
stable social secret; real successful VK, OK and Mail.ru authorization; signed
packages/download links; physical-device browser return/restart; approved public
operator documents/support and operational backups/monitoring. The existing
[RU release checklist](plans/ru-release-2026-10-28.md) retains the full gate list.
