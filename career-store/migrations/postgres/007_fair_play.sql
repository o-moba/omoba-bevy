-- Fair play (P6 / LR-33). Applied only by the migration owner, transactionally
-- under the existing migration advisory lock. Runtime registration is separate.
ALTER TABLE career_profiles ADD COLUMN abandons BIGINT NOT NULL DEFAULT 0
    CHECK (abandons BETWEEN 0 AND 4294967295);

-- One event per profile/result, in the same transaction as terminal settlement.
-- The rolling window uses database time and survives lobby/process changes.
CREATE TABLE career_abandon_events (
    profile_id TEXT NOT NULL REFERENCES career_profiles(profile_id) ON DELETE CASCADE,
    result_id TEXT NOT NULL REFERENCES career_matches(result_id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (profile_id, result_id)
);
CREATE INDEX career_abandon_events_window
    ON career_abandon_events(profile_id, created_at DESC);

CREATE TABLE career_play_penalties (
    profile_id TEXT PRIMARY KEY REFERENCES career_profiles(profile_id) ON DELETE CASCADE,
    cooldown_until TIMESTAMPTZ NOT NULL,
    strikes INTEGER NOT NULL CHECK (strikes > 0)
);

INSERT INTO career_schema_version VALUES (7);
