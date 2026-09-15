-- Separate version history: do not change career_schema_version or game receipts.
CREATE SCHEMA IF NOT EXISTS portal;
CREATE TABLE portal.schema_version(version integer PRIMARY KEY);
INSERT INTO portal.schema_version VALUES(1);
CREATE TABLE portal.web_pairings (
  pair_id text PRIMARY KEY CHECK(pair_id ~ '^[0-9a-f]{64}$'),
  code_hash text NOT NULL UNIQUE,
  poll_hash text NOT NULL,
  nonce text NOT NULL,
  expires_at bigint NOT NULL,
  state text NOT NULL DEFAULT 'pending' CHECK(state IN('pending','approved','denied','cancelled','consumed')),
  profile_id text REFERENCES public.career_profiles(profile_id),
  approved_key text,
  session_id text,
  delivery bytea,
  delivery_until bigint
);
CREATE INDEX pairings_expiry ON portal.web_pairings(expires_at);
CREATE TABLE portal.web_sessions (
  session_id text PRIMARY KEY,
  profile_id text NOT NULL REFERENCES public.career_profiles(profile_id),
  token_hash text NOT NULL UNIQUE,
  created_at bigint NOT NULL,
  last_seen bigint NOT NULL,
  expires_at bigint NOT NULL,
  revoked boolean NOT NULL DEFAULT false,
  label text NOT NULL DEFAULT 'Browser'
);
CREATE INDEX sessions_profile ON portal.web_sessions(profile_id,created_at DESC);
CREATE TABLE portal.profile_settings (
  profile_id text PRIMARY KEY REFERENCES public.career_profiles(profile_id),
  public_profile_enabled boolean NOT NULL DEFAULT false,
  public_stats_enabled boolean NOT NULL DEFAULT false,
  discoverable boolean NOT NULL DEFAULT false,
  locale text NOT NULL DEFAULT 'ru' CHECK(locale IN('ru','en')),
  timezone text NOT NULL DEFAULT 'UTC',
  version bigint NOT NULL DEFAULT 1,
  CHECK(public_profile_enabled OR (NOT public_stats_enabled AND NOT discoverable))
);
CREATE TABLE portal.operation_receipts (
  session_id text NOT NULL REFERENCES portal.web_sessions(session_id),
  operation_key text NOT NULL,
  body_hash text NOT NULL,
  response jsonb NOT NULL,
  expires_at bigint NOT NULL,
  PRIMARY KEY(session_id,operation_key)
);
CREATE TABLE portal.rate_limits (
  identity_hash text NOT NULL,
  bucket bigint NOT NULL,
  hits integer NOT NULL,
  PRIMARY KEY(identity_hash,bucket)
);
CREATE TABLE portal.audit_events (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  profile_id text REFERENCES public.career_profiles(profile_id),
  event text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX audit_expiry ON portal.audit_events(created_at);
CREATE TABLE portal.projected_results (
  result_id text PRIMARY KEY REFERENCES public.career_matches(result_id),
  version integer NOT NULL,
  source_hash text NOT NULL,
  projected_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE portal.player_match_facts (
  result_id text NOT NULL REFERENCES public.career_matches(result_id),
  player_id text NOT NULL,
  profile_id text REFERENCES public.career_profiles(profile_id),
  ended_at timestamptz NOT NULL,
  duration_ms double precision NOT NULL CHECK(duration_ms>=0),
  hero_class text NOT NULL,
  outcome text NOT NULL,
  rated boolean NOT NULL,
  won boolean,
  kills bigint NOT NULL,
  deaths bigint NOT NULL,
  assists bigint NOT NULL,
  damage_to_heroes double precision NOT NULL,
  minion_last_hits bigint NOT NULL,
  jungle_last_hits bigint NOT NULL,
  rating_before integer,
  rating_after integer,
  rating_delta integer,
  PRIMARY KEY(result_id,player_id)
);
CREATE INDEX facts_profile_date ON portal.player_match_facts(profile_id,ended_at DESC,result_id DESC);
CREATE INDEX facts_profile_class_date ON portal.player_match_facts(profile_id,hero_class,ended_at DESC);
