-- Shared ready-check/draft dodge discipline. Only trusted server events enter
-- these tables; they are independent of played-match/AFK abandon penalties.
CREATE TABLE career_queue_dodge_events (
    event_id TEXT PRIMARY KEY CHECK (event_id ~ '^[0-9a-f]{32}$'),
    profile_id TEXT NOT NULL REFERENCES career_profiles(profile_id) ON DELETE CASCADE,
    observed_at_ms BIGINT NOT NULL CHECK (observed_at_ms >= 0),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX career_queue_dodge_events_profile ON career_queue_dodge_events(profile_id);

CREATE TABLE career_queue_penalties (
    profile_id TEXT PRIMARY KEY REFERENCES career_profiles(profile_id) ON DELETE CASCADE,
    last_observed_ms BIGINT NOT NULL CHECK (last_observed_ms >= 0),
    cooldown_until TIMESTAMPTZ NOT NULL,
    strikes INTEGER NOT NULL CHECK (strikes > 0)
);

INSERT INTO career_schema_version VALUES (11);
