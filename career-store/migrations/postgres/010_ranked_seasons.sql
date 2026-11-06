-- First season starts without resetting ratings, lifetime totals or eligibility.
-- Production rollout must drain old workers before switching schema/runtime.
CREATE TABLE career_ranked_seasons (
    season_id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY CHECK (season_id > 0),
    started_at_ms BIGINT NOT NULL CHECK (started_at_ms >= 0),
    ends_at_ms BIGINT NOT NULL,
    closed_at_ms BIGINT,
    CHECK (ends_at_ms = started_at_ms + 7776000000),
    CHECK (closed_at_ms IS NULL OR closed_at_ms >= ends_at_ms),
    UNIQUE (season_id,started_at_ms,ends_at_ms)
);
CREATE UNIQUE INDEX career_one_open_ranked_season ON career_ranked_seasons ((true))
    WHERE closed_at_ms IS NULL;
WITH clock AS (SELECT floor(extract(epoch FROM clock_timestamp())*1000)::bigint AS ms)
INSERT INTO career_ranked_seasons(started_at_ms,ends_at_ms)
SELECT ms,ms+7776000000 FROM clock;

ALTER TABLE career_profiles
    ADD COLUMN ranked_season_id BIGINT,
    ADD COLUMN ranked_started_at_ms BIGINT,
    ADD COLUMN ranked_ends_at_ms BIGINT,
    ADD COLUMN season_rated_matches BIGINT NOT NULL DEFAULT 0 CHECK (season_rated_matches BETWEEN 0 AND 4294967295),
    ADD COLUMN season_wins BIGINT NOT NULL DEFAULT 0 CHECK (season_wins BETWEEN 0 AND 4294967295),
    ADD COLUMN season_losses BIGINT NOT NULL DEFAULT 0 CHECK (season_losses BETWEEN 0 AND 4294967295),
    ADD COLUMN season_completed_matches BIGINT NOT NULL DEFAULT 0 CHECK (season_completed_matches BETWEEN 0 AND 4294967295),
    ADD COLUMN latest_ranked_badge JSONB CHECK (latest_ranked_badge IS NULL OR jsonb_typeof(latest_ranked_badge)='object');
UPDATE career_profiles p SET ranked_season_id=s.season_id,
    ranked_started_at_ms=s.started_at_ms,ranked_ends_at_ms=s.ends_at_ms
FROM career_ranked_seasons s WHERE s.closed_at_ms IS NULL;
ALTER TABLE career_profiles
    ALTER COLUMN ranked_season_id SET NOT NULL,
    ALTER COLUMN ranked_started_at_ms SET NOT NULL,
    ALTER COLUMN ranked_ends_at_ms SET NOT NULL,
    ADD CONSTRAINT career_profile_ranked_window FOREIGN KEY
        (ranked_season_id,ranked_started_at_ms,ranked_ends_at_ms)
        REFERENCES career_ranked_seasons(season_id,started_at_ms,ends_at_ms),
    ADD CONSTRAINT career_profile_season_totals CHECK (
        season_rated_matches<=rated_matches AND season_wins+season_losses<=season_rated_matches
        AND season_completed_matches<=season_rated_matches);
CREATE INDEX career_profiles_season_ladder
    ON career_profiles(rating DESC,season_rated_matches DESC,profile_id)
    WHERE season_rated_matches>=20;

-- All registration routes, including portal SQL, coordinate with rollover.
-- The narrow owner trigger locks metadata without granting runtime UPDATE.
-- Pin the trusted schema before pg_temp; caller-created temp tables cannot
-- replace the authoritative window during registration.
CREATE FUNCTION career_stamp_ranked_profile() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER AS $$
DECLARE current_season career_ranked_seasons%ROWTYPE;
BEGIN
    PERFORM pg_advisory_xact_lock_shared(721946010);
    SELECT * INTO STRICT current_season FROM career_ranked_seasons WHERE closed_at_ms IS NULL FOR SHARE;
    NEW.ranked_season_id := current_season.season_id;
    NEW.ranked_started_at_ms := current_season.started_at_ms;
    NEW.ranked_ends_at_ms := current_season.ends_at_ms;
    NEW.season_rated_matches := 0;
    NEW.season_wins := 0;
    NEW.season_losses := 0;
    NEW.season_completed_matches := 0;
    NEW.latest_ranked_badge := NULL;
    RETURN NEW;
END $$;
DO $$
BEGIN
    EXECUTE format('ALTER FUNCTION %I.career_stamp_ranked_profile() SET search_path TO %I, pg_temp',current_schema(),current_schema());
END $$;
REVOKE ALL ON FUNCTION career_stamp_ranked_profile() FROM PUBLIC;
CREATE TRIGGER career_ranked_registration BEFORE INSERT ON career_profiles
    FOR EACH ROW EXECUTE FUNCTION career_stamp_ranked_profile();

-- Allocation intent owns the season even when the final result is unrated.
CREATE TABLE career_ranked_allocations (
    result_id TEXT PRIMARY KEY REFERENCES career_matches(result_id),
    season_id BIGINT NOT NULL REFERENCES career_ranked_seasons(season_id)
);
INSERT INTO career_ranked_allocations(result_id,season_id)
SELECT m.result_id,s.season_id FROM career_matches m CROSS JOIN career_ranked_seasons s
WHERE m.status!='settled' AND m.allocation->>'rated'='true' AND s.closed_at_ms IS NULL;
CREATE TRIGGER career_ranked_allocation_guard BEFORE INSERT OR UPDATE OR DELETE
    ON career_ranked_allocations FOR EACH ROW EXECUTE FUNCTION career_guard_participant();

CREATE TABLE career_ranked_archives (
    season_id BIGINT NOT NULL REFERENCES career_ranked_seasons(season_id),
    profile_id TEXT NOT NULL REFERENCES career_profiles(profile_id),
    rating INTEGER NOT NULL CHECK (rating BETWEEN 0 AND 5000),
    rated_matches BIGINT NOT NULL CHECK (rated_matches BETWEEN 1 AND 4294967295),
    wins BIGINT NOT NULL CHECK (wins >= 0),
    losses BIGINT NOT NULL CHECK (losses >= 0),
    completed_matches BIGINT NOT NULL CHECK (completed_matches BETWEEN 0 AND 4294967295),
    badge JSONB CHECK (badge IS NULL OR jsonb_typeof(badge)='object'),
    PRIMARY KEY (season_id,profile_id),
    CHECK (wins+losses<=rated_matches AND completed_matches<=rated_matches)
);
CREATE INDEX career_ranked_badge_history ON career_ranked_archives(profile_id,season_id DESC);
CREATE TABLE career_ranked_closures (
    season_id BIGINT PRIMARY KEY REFERENCES career_ranked_seasons(season_id),
    next_season_id BIGINT NOT NULL UNIQUE REFERENCES career_ranked_seasons(season_id),
    closed_at_ms BIGINT NOT NULL CHECK (closed_at_ms >= 0),
    archived_profiles BIGINT NOT NULL CHECK (archived_profiles >= 0),
    awarded_badges BIGINT NOT NULL CHECK (awarded_badges BETWEEN 0 AND archived_profiles),
    CHECK (season_id!=next_season_id)
);
CREATE FUNCTION career_guard_ranked_archive() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'ranked archive and close receipts are immutable' USING ERRCODE='23514';
END $$;
CREATE TRIGGER career_ranked_archive_guard BEFORE UPDATE OR DELETE ON career_ranked_archives
    FOR EACH ROW EXECUTE FUNCTION career_guard_ranked_archive();
CREATE TRIGGER career_ranked_close_guard BEFORE UPDATE OR DELETE ON career_ranked_closures
    FOR EACH ROW EXECUTE FUNCTION career_guard_ranked_archive();
CREATE FUNCTION career_guard_ranked_season() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' OR OLD.closed_at_ms IS NOT NULL
       OR NEW.season_id IS DISTINCT FROM OLD.season_id
       OR NEW.started_at_ms IS DISTINCT FROM OLD.started_at_ms
       OR NEW.ends_at_ms IS DISTINCT FROM OLD.ends_at_ms
       OR NEW.closed_at_ms IS NULL THEN
        RAISE EXCEPTION 'ranked season identity and closed seasons are immutable' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER career_ranked_season_guard BEFORE UPDATE OR DELETE ON career_ranked_seasons
    FOR EACH ROW EXECUTE FUNCTION career_guard_ranked_season();


-- A close may temporarily have no open row inside its transaction. At commit
-- it must have exactly one replacement; incidental SQL cannot strand admission.
CREATE FUNCTION career_require_open_ranked_season() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF (SELECT count(*) FROM career_ranked_seasons WHERE closed_at_ms IS NULL)!=1 THEN
        RAISE EXCEPTION 'exactly one ranked season must remain open' USING ERRCODE='23514';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER career_ranked_open_window
    AFTER INSERT OR UPDATE OR DELETE ON career_ranked_seasons
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
    EXECUTE FUNCTION career_require_open_ranked_season();

INSERT INTO career_schema_version(version) VALUES (10);
