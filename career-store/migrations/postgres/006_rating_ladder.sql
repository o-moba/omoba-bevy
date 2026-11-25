-- Rating ladder (LR-47): the leaderboard reads the top profiles by rating. The
-- partial index covers only profiles past the newcomer bracket, which is the
-- ladder's own filter, so the leaderboard query is an index range scan.
-- Executed transactionally under the adapter's migration advisory lock.
CREATE INDEX career_profiles_ladder ON career_profiles(rating DESC, rated_matches DESC, profile_id)
    WHERE rated_matches >= 20;
INSERT INTO career_schema_version VALUES (6);
