-- Admission limits (LR-16): a durable counter of profiles created for new device
-- keys, per source and in total, and retention of profiles that never played.
-- Executed transactionally under the adapter's migration advisory lock.

-- One random salt hashes source addresses: no row holds a raw IP address.
CREATE TABLE career_admission_salt (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    salt TEXT NOT NULL DEFAULT gen_random_uuid()::text
);
INSERT INTO career_admission_salt DEFAULT VALUES;

-- One row per profile created for a new device key. The limits count the last
-- 24 hours; `CareerStore::prune_unplayed_profiles` deletes rows after 48 hours.
CREATE TABLE career_profile_admissions (
    seq BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    source TEXT NOT NULL CHECK (source ~ '^[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX career_profile_admissions_recent ON career_profile_admissions(created_at);
CREATE INDEX career_profile_admissions_source ON career_profile_admissions(source, created_at);

-- Retention: deletes profiles that never played and that nothing else holds.
-- Runs with the owner's rights, so the game role still cannot delete device keys
-- (`account-api/ops/grants.sql`); only this function decides which keys go. A
-- profile is kept when it has another or a revoked key, a match, a friend, a live
-- presence, or any row elsewhere (portal sessions, sign-in links, Supporter
-- grants): that row's foreign key refuses the delete and the profile stays.
CREATE FUNCTION career_prune_unplayed_profiles(min_age_days integer, max_profiles integer)
RETURNS integer LANGUAGE plpgsql SECURITY DEFINER AS $$
DECLARE
    candidate record;
    pruned integer := 0;
BEGIN
    IF min_age_days < 7 OR max_profiles < 1 OR max_profiles > 500 THEN
        RAISE EXCEPTION 'retention needs at least 7 days and 1-500 profiles per run'
            USING ERRCODE = '22023';
    END IF;
    FOR candidate IN
        SELECT p.profile_id, k.public_key
        FROM career_profiles p JOIN career_keys k USING (profile_id)
        WHERE p.matches_played = 0 AND p.rated_matches = 0 AND p.progression_xp = 0
          AND k.revoked_at IS NULL
          AND k.created_at < clock_timestamp() - make_interval(days => min_age_days)
          AND NOT EXISTS (SELECT 1 FROM career_keys o
                          WHERE o.profile_id = p.profile_id AND o.public_key <> k.public_key)
          AND NOT EXISTS (SELECT 1 FROM career_participants x WHERE x.profile_id = p.profile_id)
          AND NOT EXISTS (SELECT 1 FROM career_active_profiles a WHERE a.profile_id = p.profile_id)
          AND NOT EXISTS (SELECT 1 FROM career_friendships f
                          WHERE f.low_id = p.profile_id OR f.high_id = p.profile_id)
          AND NOT EXISTS (SELECT 1 FROM career_presence s
                          WHERE s.profile_id = p.profile_id AND s.expires_at > clock_timestamp())
        ORDER BY k.created_at
        LIMIT max_profiles
    LOOP
        BEGIN
            -- The lock a sign-in of this key takes: never delete under a login.
            PERFORM pg_advisory_xact_lock(hashtextextended(candidate.public_key, 721946002));
            PERFORM 1 FROM career_profiles
                WHERE profile_id = candidate.profile_id AND matches_played = 0 FOR UPDATE;
            IF NOT FOUND OR EXISTS (SELECT 1 FROM career_presence s
                    WHERE s.profile_id = candidate.profile_id AND s.expires_at > clock_timestamp()) THEN
                CONTINUE;
            END IF;
            DELETE FROM career_presence WHERE profile_id = candidate.profile_id;
            DELETE FROM career_keys WHERE profile_id = candidate.profile_id;
            DELETE FROM career_profiles WHERE profile_id = candidate.profile_id;
            pruned := pruned + 1;
        EXCEPTION WHEN foreign_key_violation THEN
            -- Linked elsewhere: not an unlinked profile, so it stays.
            NULL;
        END;
    END LOOP;
    RETURN pruned;
END $$;
-- A SECURITY DEFINER function must not resolve names through a schema its caller
-- controls: pin the migration's own schemas, with pg_temp last.
DO $$
BEGIN
    EXECUTE format(
        'ALTER FUNCTION career_prune_unplayed_profiles(integer, integer) SET search_path = %s, pg_temp',
        (SELECT string_agg(quote_ident(name), ', ') FROM unnest(current_schemas(false)) AS name));
END $$;
REVOKE ALL ON FUNCTION career_prune_unplayed_profiles(integer, integer) FROM PUBLIC;
INSERT INTO career_schema_version VALUES(4);
