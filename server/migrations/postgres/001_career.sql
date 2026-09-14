-- Executed transactionally under the adapter's migration advisory lock.
CREATE TABLE career_profiles (
    profile_id TEXT PRIMARY KEY CHECK (profile_id ~ '^[0-9a-f]{64}$'),
    nickname TEXT NOT NULL CHECK (char_length(nickname) BETWEEN 1 AND 20),
    rating INTEGER NOT NULL DEFAULT 1000 CHECK (rating BETWEEN 0 AND 5000),
    rated_matches BIGINT NOT NULL DEFAULT 0 CHECK (rated_matches BETWEEN 0 AND 4294967295),
    matches_played BIGINT NOT NULL DEFAULT 0 CHECK (matches_played BETWEEN 0 AND 4294967295),
    wins BIGINT NOT NULL DEFAULT 0 CHECK (wins BETWEEN 0 AND 4294967295),
    losses BIGINT NOT NULL DEFAULT 0 CHECK (losses BETWEEN 0 AND 4294967295),
    progression_xp BIGINT NOT NULL DEFAULT 0 CHECK (progression_xp >= 0),
    CHECK (rated_matches <= matches_played AND wins + losses <= matches_played)
);
CREATE TABLE career_keys (
    public_key TEXT PRIMARY KEY CHECK (public_key ~ '^[0-9a-f]{64}$'),
    profile_id TEXT NOT NULL REFERENCES career_profiles(profile_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE career_matches (
    seq BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    result_id TEXT NOT NULL UNIQUE CHECK (char_length(result_id) BETWEEN 1 AND 128),
    server_epoch TEXT NOT NULL CHECK (server_epoch ~ '^[1-9][0-9]{0,19}$'
        AND (length(server_epoch) < 20 OR server_epoch <= '18446744073709551615')),
    match_id TEXT NOT NULL CHECK (match_id ~ '^[1-9][0-9]{0,19}$'
        AND (length(match_id) < 20 OR match_id <= '18446744073709551615')),
    owner_id TEXT NOT NULL,
    generation BIGINT NOT NULL DEFAULT 1 CHECK (generation > 0),
    lease_until TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running','pending','settled')),
    allocation JSONB NOT NULL CHECK (jsonb_typeof(allocation) = 'object'),
    checkpoint JSONB NOT NULL CHECK (jsonb_typeof(checkpoint) = 'object'),
    intent JSONB CHECK (jsonb_typeof(intent) = 'object'),
    result JSONB CHECK (jsonb_typeof(result) = 'object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    settled_at TIMESTAMPTZ,
    CHECK ((status = 'running' AND intent IS NULL AND result IS NULL)
        OR (status = 'pending' AND intent IS NOT NULL AND result IS NULL)
        OR (status = 'settled' AND intent IS NOT NULL AND result IS NOT NULL))
);
CREATE INDEX career_pending_matches ON career_matches(lease_until, seq) WHERE status != 'settled';
CREATE TABLE career_participants (
    result_id TEXT NOT NULL REFERENCES career_matches(result_id),
    seat SMALLINT NOT NULL CHECK (seat BETWEEN 0 AND 31),
    player_id TEXT NOT NULL CHECK (player_id ~ '^[1-9][0-9]{0,19}$'
        AND (length(player_id) < 20 OR player_id <= '18446744073709551615')),
    profile_id TEXT REFERENCES career_profiles(profile_id),
    PRIMARY KEY (result_id, seat),
    UNIQUE (result_id, player_id),
    UNIQUE (result_id, profile_id)
);
CREATE INDEX career_profile_history ON career_participants(profile_id, result_id);
-- An assignment is retained until settlement, including disconnect and DB retry.
CREATE TABLE career_active_profiles (
    profile_id TEXT PRIMARY KEY REFERENCES career_profiles(profile_id),
    result_id TEXT NOT NULL REFERENCES career_matches(result_id)
);
CREATE TABLE career_friendships (
    low_id TEXT NOT NULL REFERENCES career_profiles(profile_id),
    high_id TEXT NOT NULL REFERENCES career_profiles(profile_id),
    requested_by TEXT NOT NULL REFERENCES career_profiles(profile_id),
    accepted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (low_id, high_id),
    CHECK (low_id < high_id AND requested_by IN (low_id, high_id))
);
CREATE INDEX career_friendships_high ON career_friendships(high_id, low_id);
CREATE TABLE career_presence (
    profile_id TEXT PRIMARY KEY REFERENCES career_profiles(profile_id),
    owner_id TEXT NOT NULL,
    result_id TEXT REFERENCES career_matches(result_id),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE FUNCTION career_guard_match() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'career matches are append-only' USING ERRCODE = '23514';
    END IF;
    IF OLD.status = 'settled' THEN
        RAISE EXCEPTION 'settled result is immutable' USING ERRCODE = '23514';
    END IF;
    IF NEW.seq IS DISTINCT FROM OLD.seq OR NEW.result_id IS DISTINCT FROM OLD.result_id
       OR NEW.server_epoch IS DISTINCT FROM OLD.server_epoch OR NEW.match_id IS DISTINCT FROM OLD.match_id
       OR NEW.allocation IS DISTINCT FROM OLD.allocation THEN
        RAISE EXCEPTION 'allocation identity is immutable' USING ERRCODE = '23514';
    END IF;
    IF OLD.intent IS NOT NULL AND NEW.intent IS DISTINCT FROM OLD.intent THEN
        RAISE EXCEPTION 'settlement intent is immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER career_match_guard BEFORE UPDATE OR DELETE ON career_matches
    FOR EACH ROW EXECUTE FUNCTION career_guard_match();

CREATE FUNCTION career_guard_participant() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE current_status TEXT;
BEGIN
    -- Every path takes this lock: the state check cannot race finalization.
    SELECT status INTO current_status FROM career_matches
        WHERE result_id = COALESCE(NEW.result_id, OLD.result_id) FOR UPDATE;
    IF TG_OP != 'INSERT' OR current_status != 'running' THEN
        RAISE EXCEPTION 'participant identities are append-only while running' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER career_participant_guard BEFORE INSERT OR UPDATE OR DELETE ON career_participants
    FOR EACH ROW EXECUTE FUNCTION career_guard_participant();
