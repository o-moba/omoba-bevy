-- Server-owned audit and provider periods; clients never write grants directly.
CREATE TABLE portal.supporter_accounts (
  profile_id text PRIMARY KEY REFERENCES public.career_profiles(profile_id),
  apple_account_token text NOT NULL UNIQUE CHECK(apple_account_token ~ '^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$')
);
CREATE TABLE portal.supporter_events (
  provider text NOT NULL CHECK(provider IN ('apple','solana')),
  event_id text NOT NULL,
  profile_id text NOT NULL REFERENCES public.career_profiles(profile_id),
  period_id text NOT NULL,
  event_version bigint NOT NULL,
  payload_hash text NOT NULL,
  payload jsonb NOT NULL,
  received_at bigint NOT NULL,
  PRIMARY KEY(provider,event_id)
);
CREATE TABLE portal.supporter_grants (
  provider text NOT NULL CHECK(provider IN ('apple','solana')),
  period_id text NOT NULL,
  profile_id text NOT NULL REFERENCES public.career_profiles(profile_id),
  original_transaction_id text,
  valid_from bigint NOT NULL,
  valid_until bigint NOT NULL CHECK(valid_until>valid_from),
  revoked_at bigint,
  renewal_enabled boolean,
  last_reconciled_at bigint NOT NULL DEFAULT 0,
  event_version bigint NOT NULL,
  PRIMARY KEY(provider,period_id)
);
CREATE INDEX supporter_active_profile ON portal.supporter_grants(profile_id,valid_until DESC);
CREATE INDEX supporter_original_subscription ON portal.supporter_grants(provider,original_transaction_id);
CREATE TABLE portal.supporter_preferences (
  profile_id text PRIMARY KEY REFERENCES public.career_profiles(profile_id),
  aura text CHECK(aura IN ('solar','lunar','verdant'))
);
CREATE TABLE portal.supporter_orders (
  order_id text PRIMARY KEY,
  profile_id text NOT NULL REFERENCES public.career_profiles(profile_id),
  reference text NOT NULL UNIQUE,
  network text NOT NULL,
  genesis_hash text NOT NULL,
  mint text NOT NULL,
  recipient text NOT NULL,
  amount bigint NOT NULL CHECK(amount>0),
  decimals integer NOT NULL CHECK(decimals BETWEEN 0 AND 9),
  created_at bigint NOT NULL,
  expires_at bigint NOT NULL CHECK(expires_at>created_at),
  confirmed_signature text UNIQUE,
  confirmed_at bigint
);
CREATE INDEX supporter_orders_profile ON portal.supporter_orders(profile_id,expires_at);
CREATE TABLE portal.supporter_nonces (
  public_key text NOT NULL,
  nonce text NOT NULL,
  expires_at bigint NOT NULL,
  PRIMARY KEY(public_key,nonce)
);
INSERT INTO portal.schema_version VALUES(3);
