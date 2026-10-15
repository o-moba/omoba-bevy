# Shared accounts and Hero Supporter

Hero Supporter is voluntary cosmetic account access. No purchase modifies damage, health, XP,
rating, matchmaking, visibility rules or hitboxes. Solar, Lunar and Verdant use
the same lightweight mesh-based effect with separate color presentation.

## Delivery status

Implemented: native device enrollment/recovery, portal device management,
server-authoritative aura selection/replication, native and web preview, persisted
provider events/grants, Solana checkout validation, StoreKit purchase/restore
bridge, Apple verification boundary and reconciliation worker.

Payment providers are disabled without complete operator configuration. Local
tests use synthetic accounts and provider responses. They do not establish a
successful Apple sandbox purchase, Solana devnet wallet payment, physical-phone
usability or App Review acceptance. The Apple verification policy is under
`billing/apple-verifier/`; binding it to Apple's official server library requires
the separately approved production dependency. Do not enable Apple checkout with
a mock verifier or a service that only decodes JWT payloads.

## Monthly Hero status and public thanks

Hero is the player-facing name for the existing Supporter cosmetic entitlement.
It is not a combat rank or a separate client-granted right. A verified active
period unlocks the Solar, Lunar and Verdant aura styles; hiding an aura does not
end the paid period. The server continues to own authorization and expiry.

On the website, Solana USDC buys **30 days of prepaid support**, without automatic
wallet debits. The existing Apple product must be configured as a **monthly**
subscription; its renewal and cancellation rules come from StoreKit. Do not call
the website payment an automatically renewing subscription. Both are voluntary
ways to support the project and neither grants gameplay advantages.

The portal shows `months_supported`, the count of unique verified, nonrevoked
monthly payment periods across providers. Expired periods and already-paid future
periods retain their credit; a refund removes that period's credit. Retries,
restores and repeated provider events never add a second month. This count is
queried from the entire grant ledger, independently of the bounded recent-history
list. It is not computed by dividing elapsed days by 30: calendar months and
Apple sandbox renewal intervals differ. A future annual product would need an
explicit period-credit policy before it could be sold through this mechanism.

Public thanks are optional. The account owner enables acknowledgement on the
Supporter page and separately publishes their game profile in Settings. The
public `/supporters` directory includes all profiles meeting both conditions
with at least one credited month, using bounded cursor pagination. It shows the
nickname, credited months, whether Hero is currently active, and a link to the
existing player profile. Former supporters remain acknowledged after expiry.
Opting out or making the profile private removes public acknowledgement. Public
statistics remain independently controlled by the existing statistics setting;
no payment publishes hidden statistics. Public responses never contain receipts,
wallets, billing sources or renewal settings.

The Account API adds `recognition` to the existing private supporter dashboard,
`PATCH /v1/me/supporter/recognition` for its explicit visibility preference, and
public `GET /v1/supporters`. Public player responses include a small supporter
summary only when that same acknowledgement is public. Existing game/native
SupporterStatus and network messages are unchanged.

## One account on phone and PC

1. On an existing device, connect the player portal through Profile. The game
   explicitly asks permission to manage devices, recovery and cosmetics as well
   as friends/profile settings. Confirm only your own browser's login.
2. On the new native device, open Profile → Link/recover account. Generate an
   enrollment and leave the original installation key untouched.
3. In the portal's Devices page, enter its eight-character code. Compare the
   first/last eight hexadecimal key characters and review the target nickname.
   Approve only your own device. Privileged portal actions require confirmation
   within ten minutes.
4. On the new device, refresh, review the target account and explicitly confirm.
   Restart to activate it. Existing profiles are never merged; the former local
   private key is preserved in a private backup in the preferences directory.

Each installation keeps its own Ed25519 key. A new key is enrolled only after
proof of possession and account-owner authorization. Existing/previously revoked
keys cannot be reassigned. Native enrollment uses a separate signature domain
from web pairing and gameplay messages, and expires after five minutes.

The Devices page generates eight single-use 256-bit recovery codes. Store them
in a password manager: plaintext is shown once, never saved by the website, and
the database stores keyed hashes. A new set invalidates unused old codes. To
recover, enter one code in the new device's recovery screen, confirm the named
account and restart. Afterwards review Devices and revoke lost devices. Recovery
does not silently delete other valid native devices.

Revocation retains a key tombstone, ends browser sessions and cancels pending
approvals. The final native key cannot be revoked without an unused recovery
code. Online game keys are rechecked every ten seconds and fail closed after
thirty seconds without a successful check; no silent fallback to a guest account.

## Database upgrade and runtime permissions

Core career schema is version 3; portal schema is independently version 6.
The new `006_supporter_recognition.sql` migration adds public acknowledgement
with an opt-out default; it does not grant paid access or publish any account.
Run the existing owner-only Account API `migrate` command, then
`account-api/ops/grants.sql` with your existing runtime role names. Apply first
to an isolated database and verify backups before an operator-approved rollout.
Application startup never performs DDL. No Prisma/Drizzle schema push is used.

Portal v2 invalidates legacy browser sessions/pairings and records authorization
version 2 only for the new signed seven-scope consent. An older still-running
replica cannot mint a legacy-scope session that gains device/recovery authority.
Deploy matching clients before expecting users to reauthorize the portal.

The portal may insert a new proved career key and update its revocation date,
but cannot reassign or delete it or change account XP/rating. The game role reads
grants and writes only cosmetic preferences. It cannot issue payments/grants.
Legacy broad update/delete permissions on career keys are removed by the grants
script. Missing billing tables/permissions disable cosmetics; do not interpret
that condition as a successful provider configuration.

## Payment periods and account ownership

`portal.supporter_events` is the append-only runtime audit. A grant is identified
by provider and payment period, attached to an immutable career profile ID.
Apple's original subscription and account token cannot migrate to a second
profile during restore. A wallet address or editable nickname is not an account
authorization credential.

Duplicate events do not extend access. Older events cannot roll back current
periods; a verified refund permanently revokes its period. Cancellation of future
renewal retains the already-paid period. Unknown renewal state is distinct from
false. A second provider's valid period survives a refund of the first provider.
Refund reversals need explicit support/reconciliation policy; this implementation
does not automatically revive a revoked period. Billing grace-period extension
is not granted beyond the verified paid expiration.

Server snapshots authorize the aura, not client booleans. Grants refresh every
ten seconds, cache validity is bounded to thirty seconds, and verified expiration
cuts off without waiting for refresh. Cosmetic rendering follows visible living
actors; the preview uses an isolated render layer and cannot equip a world actor.

## Solana website provider

Supply all of these only when enabling the provider:

- `OMOBA_SOLANA_RPC_URL`: trusted HTTPS RPC endpoint; no redirect/proxy following.
- `OMOBA_SOLANA_NETWORK`: `devnet` or `mainnet-beta`.
- `OMOBA_SOLANA_GENESIS_HASH`: canonical genesis hash for that network.
- `OMOBA_SOLANA_MINT`: canonical Circle USDC mint on that network.
- `OMOBA_SOLANA_TREASURY`: intended recipient wallet public key.
- `OMOBA_SOLANA_AMOUNT_ATOMIC`: price in integer millionths of USDC.

The adapter pins the network/genesis/mint mapping and six decimals. The quote
lives fifteen minutes and belongs to the authenticated account. Solana is a
30-day prepayment, not permission to debit a wallet again. Wallet network fees
are additional. For devnet, explicitly switch the wallet to Devnet before paying;
the transfer URI does not select a network for the wallet.

Confirmation fetches finalized RPC data and checks success, canonical token
program, mint, exact atomic amount, treasury ownership and balance gain,
reference placement, signature uniqueness and the original quote time window.
Browser-supplied JSON transaction data cannot grant access. Expired quotes remain
confirmable if payment happened within their window. The account's recent orders
remain available after reload; never pay again just because confirmation is slow.

## Apple provider boundary

The Swift StoreKit 2 bridge is compiled automatically by `client/build.rs` on
iOS, for device and simulator targets. Desktop/Android do not link StoreKit.
Existing native build scripts remain the entry points. Configure public API and
portal HTTPS origins at compile time as documented by the Account API; no server
secret belongs in a mobile binary.

The native client obtains an account-bound UUID token, displays localized StoreKit
monthly pricing, and passes that token through `Product.purchase`. It supports
pending/Ask to Buy, cancellation, transaction updates and Restore purchases.
`Transaction.finish()` occurs only after the server commits verification. Failed
receipts remain unfinished and are retried independently; a failed price request
does not permanently disable restore. Active or unknown account entitlement
disables the purchase button while restore stays available.

The Rust API requires `OMOBA_APPLE_VERIFIER_URL` (loopback HTTP only),
`OMOBA_APPLE_VERIFIER_SECRET` (private 32-byte hex shared secret),
`OMOBA_APPLE_PRODUCT_ID`, and `OMOBA_APPLE_ENVIRONMENT` (`Sandbox` or `Production`).
Missing/partial configuration fails closed. The secret is never a client value.

Verifier contract: authenticated `POST /verify`, JSON
`{kind:"transaction"|"notification",signed_payload:"..."}` or
`{kind:"reconcile",original_transaction_id:"..."}`. The response is a bounded
`events` array of verified normalized periods. Apple signatures, certificate chain,
bundle ID, environment, product and account token must be verified before output.
Restore fetches current provider transaction state so a historical valid JWS
cannot undo a refund. Notifications verify both outer and nested signed data.
The API periodically reconciles bounded batches to recover missed notifications.

Real rollout additionally needs App Store Connect product/group setup, approved
pricing, public privacy/terms links in the purchase experience and metadata,
server API credentials/root certificates, an HTTPS notification route, and actual
sandbox tests of purchase, pending approval, renewal, restore, cancellation,
refund and reinstallation. None of those operator/account actions happen merely
by compiling this code. No crypto-purchase link is added to the iOS purchase UI.

## Verification

Use the task evidence under `.agent/tasks/SUPPORTER-2026-09-16/`. Rust tests cover
proof binding, device races, recovery/revocation, provider replay/order/refund,
receipt ownership, RPC validation and cosmetic authorization. PostgreSQL tests
require an explicitly isolated owner URL and include restricted runtime roles.
`omoba-web/tests/supporter.e2e.mjs` uses a synthetic seed7 account against only the
documented disposable localhost fixture. Never run fixtures on production.

Native screenshots are enabled only by `OMOBA_QA_SUPPORTER=1` with
`OMOBA_SUPPORTER_QA_DIR` pointing to a disposable output directory. This opens a
local preview; it never creates an account entitlement or a paid world aura.
