-- Rating ladder listing (LR-47, portal schema 11). GET /v1/leaderboard shows a
-- profile only while this flag holds. It is an opt-out, not an opt-in: a player's
-- nickname and rating already reach every opponent through match reports, so the
-- ladder reveals nothing a match did not. A profile without a settings row counts
-- as listed; the API reads coalesce(leaderboard_listed, true).
ALTER TABLE portal.profile_settings ADD COLUMN leaderboard_listed boolean NOT NULL DEFAULT true;
INSERT INTO portal.schema_version VALUES(11);
