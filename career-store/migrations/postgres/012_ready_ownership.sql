-- Durable ownership precedes publishing consent. Closed rows retain exact
-- immutable membership/response receipts; claims alone gate another search.
CREATE TABLE career_ready_checks (
    check_id TEXT PRIMARY KEY CHECK (check_id ~ '^[0-9a-f]{32}$'),
    owner_id TEXT NOT NULL,
    queue TEXT NOT NULL CHECK (queue IN ('quick','humans_only','bot_practice')),
    member_count INTEGER NOT NULL CHECK (member_count BETWEEN 1 AND 10),
    state TEXT NOT NULL CHECK (state IN ('forming','open','approved','cancelled','expired','consumed')),
    opened_at_ms BIGINT CHECK (opened_at_ms >= 0),
    deadline_ms BIGINT NOT NULL CHECK (deadline_ms >= 0),
    live_until_ms BIGINT NOT NULL CHECK (live_until_ms >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE career_ready_members (
    check_id TEXT NOT NULL REFERENCES career_ready_checks(check_id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES career_profiles(profile_id) ON DELETE CASCADE,
    session_id TEXT NOT NULL CHECK (length(session_id) BETWEEN 1 AND 64 AND session_id ~ '^[A-Za-z0-9._-]+$'),
    accepted BOOLEAN NOT NULL DEFAULT false,
    dodged BOOLEAN NOT NULL DEFAULT false,
    dodge_event_id TEXT NOT NULL UNIQUE DEFAULT replace(gen_random_uuid()::text,'-','') CHECK (dodge_event_id ~ '^[0-9a-f]{32}$'),
    dodge_observed_ms BIGINT CHECK (dodge_observed_ms >= 0),
    PRIMARY KEY(check_id,profile_id),
    UNIQUE(check_id,session_id),
    CHECK (dodged = (dodge_observed_ms IS NOT NULL))
);
CREATE INDEX career_ready_members_profile ON career_ready_members(profile_id);
CREATE TABLE career_ready_claims (
    profile_id TEXT PRIMARY KEY REFERENCES career_profiles(profile_id) ON DELETE CASCADE,
    check_id TEXT NOT NULL,
    FOREIGN KEY(check_id,profile_id) REFERENCES career_ready_members(check_id,profile_id) ON DELETE CASCADE
);
CREATE INDEX career_ready_claims_check ON career_ready_claims(check_id);
INSERT INTO career_schema_version VALUES (12);
