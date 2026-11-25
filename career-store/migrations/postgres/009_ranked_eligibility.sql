-- Owner decision 2026-11-06: ten completed public games, no VK requirement.
-- Separate from matches_played, which includes played interruptions/forfeits.
ALTER TABLE career_profiles ADD COLUMN completed_public_matches BIGINT NOT NULL DEFAULT 0
    CHECK (completed_public_matches BETWEEN 0 AND 4294967295);

-- Only trusted settled results; absent legacy participation is not an abandon.
WITH completed AS (
    SELECT p.profile_id, count(*) AS total
    FROM career_matches m
    CROSS JOIN LATERAL jsonb_array_elements(m.result->'participants') AS seat
    JOIN career_profiles p ON p.profile_id=seat->>'profile_id'
    WHERE m.status='settled'
      AND m.result->>'outcome'='completed'
      AND COALESCE((m.result->>'gameplay_started')::boolean,true)
      AND (m.result->>'ruleset'='public-casual-v1'
           OR (m.result->>'ruleset'='verdant-default-v1' AND (m.result->>'rated')::boolean))
      AND NOT COALESCE((seat->>'is_bot')::boolean,false)
      AND NOT COALESCE((seat->'participation'->>'abandoned')::boolean,false)
    GROUP BY p.profile_id
)
UPDATE career_profiles p SET completed_public_matches=c.total
FROM completed c WHERE c.profile_id=p.profile_id;

INSERT INTO career_schema_version(version) VALUES (9);
