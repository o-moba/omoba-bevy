-- YooKassa supporter donations and Hero tiers (LR-67, ADR-024/ADR-025).
-- Data half: provider 'yookassa', grant amounts and provider-neutral tier
-- points, the tier sum function and the durable YooKassa tables the provider
-- flow uses. Existing grants keep tier_points 0 (no backfill); only the Account
-- API sets points, from the provider rule (YooKassa: whole rubles after refunds).
ALTER TABLE portal.supporter_events DROP CONSTRAINT supporter_events_provider_check;
ALTER TABLE portal.supporter_events ADD CONSTRAINT supporter_events_provider_check
  CHECK(provider IN ('apple','solana','yookassa'));
ALTER TABLE portal.supporter_grants DROP CONSTRAINT supporter_grants_provider_check;
ALTER TABLE portal.supporter_grants ADD CONSTRAINT supporter_grants_provider_check
  CHECK(provider IN ('apple','solana','yookassa'));
-- Kopecks paid for a YooKassa donation; NULL for every other provider.
ALTER TABLE portal.supporter_grants ADD COLUMN amount_minor bigint;
ALTER TABLE portal.supporter_grants ADD CONSTRAINT supporter_grants_amount_check
  CHECK((provider='yookassa')=(amount_minor IS NOT NULL) AND (amount_minor IS NULL OR amount_minor>0));
ALTER TABLE portal.supporter_grants ADD COLUMN tier_points integer NOT NULL DEFAULT 0;
ALTER TABLE portal.supporter_grants ADD CONSTRAINT supporter_grants_tier_points_check
  CHECK(tier_points>=0);
-- The seven tier styles (protocol 12); the base styles keep their ids.
ALTER TABLE portal.supporter_preferences DROP CONSTRAINT supporter_preferences_aura_check;
ALTER TABLE portal.supporter_preferences ADD CONSTRAINT supporter_preferences_aura_check
  CHECK(aura IN ('solar','lunar','verdant','rose','jade','azure','crimson',
                 'prism_i','prism_ii','prism_iii'));

-- The tier sum: points of the grants whose window covers `at` (epoch seconds).
-- SECURITY INVOKER: a caller still needs SELECT on supporter_grants.
CREATE FUNCTION portal.supporter_points(profile text, at bigint) RETURNS bigint
  LANGUAGE sql STABLE PARALLEL SAFE SET search_path = pg_catalog
AS $$
  SELECT coalesce(sum(g.tier_points),0)::bigint FROM portal.supporter_grants g
  WHERE g.profile_id=profile AND g.revoked_at IS NULL AND g.valid_from<=at AND g.valid_until>at
$$;
-- Credited months for public thanks: one per nonrevoked grant, except YooKassa
-- donations, which count distinct UTC calendar months with a donation.
CREATE FUNCTION portal.supporter_months(profile text) RETURNS bigint
  LANGUAGE sql STABLE PARALLEL SAFE SET search_path = pg_catalog
AS $$
  SELECT (count(*) FILTER (WHERE g.provider<>'yookassa')
        + count(DISTINCT date_trunc('month', to_timestamp(g.valid_from) AT TIME ZONE 'UTC'))
            FILTER (WHERE g.provider='yookassa'))::bigint
  FROM portal.supporter_grants g
  WHERE g.profile_id=profile AND g.revoked_at IS NULL
$$;
REVOKE ALL ON FUNCTION portal.supporter_points(text,bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION portal.supporter_months(text) FROM PUBLIC;

-- One checkout attempt. The provider idempotence key is persisted here before
-- the first POST; a lost create is replayed with the same key only while the
-- first provider attempt is younger than 23 hours, never with a new key.
CREATE TABLE portal.supporter_yookassa_attempts (
  attempt_id text PRIMARY KEY CHECK(attempt_id ~ '^[0-9a-f]{32}$'),
  profile_id text NOT NULL REFERENCES public.career_profiles(profile_id),
  -- Keyed hash of (profile, the browser's Idempotency-Key): one attempt per key.
  request_key text NOT NULL UNIQUE CHECK(request_key ~ '^[0-9a-f]{64}$'),
  -- Sent to YooKassa as Idempotence-Key on the create and every replay.
  idempotence_key text NOT NULL UNIQUE CHECK(idempotence_key ~ '^[A-Za-z0-9_-]{16,64}$'),
  provider_payment_id text UNIQUE CHECK(provider_payment_id ~ '^[A-Za-z0-9_-]{1,80}$'),
  amount_minor bigint NOT NULL CHECK(amount_minor BETWEEN 10000 AND 10000000),
  currency text NOT NULL DEFAULT 'RUB' CHECK(currency='RUB'),
  -- The «Чеки от ЮKassa» receipt sent with the create, frozen so a replay can
  -- never change buyer or tax data (customer e-mail, one item, vat_code).
  receipt jsonb NOT NULL CHECK(jsonb_typeof(receipt)='object'),
  offer_version text NOT NULL CHECK(offer_version ~ '^[A-Za-z0-9._-]{1,40}$'),
  status text NOT NULL DEFAULT 'created'
    CHECK(status IN ('created','charging','submitted','succeeded','canceled','failed')),
  -- A worker holds the attempt while it talks to YooKassa.
  lease_until bigint,
  first_provider_attempt_at bigint,
  -- Reconciliation polls the oldest-polled submitted attempts first (rotation).
  last_polled_at bigint NOT NULL DEFAULT 0,
  -- A short code only (for example provider_replay_window_expired = manual review).
  last_error text CHECK(last_error ~ '^[a-z0-9_]{1,64}$'),
  created_at bigint NOT NULL,
  updated_at bigint NOT NULL,
  settled_at bigint,
  CHECK(status NOT IN ('submitted','succeeded') OR first_provider_attempt_at IS NOT NULL),
  CHECK(status<>'succeeded' OR (provider_payment_id IS NOT NULL AND settled_at IS NOT NULL))
);
CREATE INDEX supporter_yookassa_attempts_profile
  ON portal.supporter_yookassa_attempts(profile_id,created_at DESC);
CREATE INDEX supporter_yookassa_attempts_open
  ON portal.supporter_yookassa_attempts(status,last_polled_at)
  WHERE status IN ('created','charging','submitted','failed');

-- Succeeded refunds, each settled once. A refund may arrive before its payment
-- was settled; the settlement then settles the payment first.
CREATE TABLE portal.supporter_yookassa_refunds (
  refund_id text PRIMARY KEY CHECK(refund_id ~ '^[A-Za-z0-9_-]{1,80}$'),
  provider_payment_id text NOT NULL
    REFERENCES portal.supporter_yookassa_attempts(provider_payment_id),
  amount_minor bigint NOT NULL CHECK(amount_minor>0),
  currency text NOT NULL DEFAULT 'RUB' CHECK(currency='RUB'),
  status text NOT NULL DEFAULT 'succeeded' CHECK(status='succeeded'),
  refunded_at bigint NOT NULL,
  recorded_at bigint NOT NULL
);
CREATE INDEX supporter_yookassa_refunds_payment
  ON portal.supporter_yookassa_refunds(provider_payment_id);

-- The refund scan's durable cursor: a fixed window [watermark, window_end) is
-- paged with YooKassa's opaque cursor and only then advanced, so a crash never
-- skips an unseen refund. New charges stop when last_successful_at is stale.
CREATE TABLE portal.supporter_yookassa_refund_scan (
  key text PRIMARY KEY CHECK(key='refunds'),
  watermark bigint NOT NULL,
  window_end bigint,
  cursor text CHECK(cursor ~ '^[!-~]{1,512}$'),
  lease_until bigint,
  last_successful_at bigint,
  updated_at bigint NOT NULL,
  CHECK((window_end IS NULL AND cursor IS NULL AND lease_until IS NULL)
        OR (window_end IS NOT NULL AND watermark<=window_end))
);
INSERT INTO portal.supporter_yookassa_refund_scan(key,watermark,updated_at)
  VALUES('refunds',EXTRACT(EPOCH FROM now())::bigint,EXTRACT(EPOCH FROM now())::bigint);

-- Provider objects that need an operator: a refund of an unknown payment, a
-- refund still pending, or a settlement that failed. Never a provider body.
CREATE TABLE portal.supporter_yookassa_issues (
  provider_object_id text PRIMARY KEY CHECK(provider_object_id ~ '^[A-Za-z0-9_-]{1,80}$'),
  object_type text NOT NULL CHECK(object_type IN ('payment','refund')),
  code text NOT NULL CHECK(code IN ('pending_refund','unknown_payment','settlement_failed',
                                    'amount_mismatch','replay_window_expired')),
  occurrences integer NOT NULL DEFAULT 1 CHECK(occurrences>0),
  first_seen_at bigint NOT NULL,
  last_seen_at bigint NOT NULL,
  resolved_at bigint
);
CREATE INDEX supporter_yookassa_issues_open
  ON portal.supporter_yookassa_issues(last_seen_at) WHERE resolved_at IS NULL;
INSERT INTO portal.schema_version VALUES(9);
