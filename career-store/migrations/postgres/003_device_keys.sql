-- Keep revoked public keys as tombstones: they can never register a new profile.
ALTER TABLE career_keys ADD COLUMN revoked_at timestamptz;
ALTER TABLE career_keys ADD COLUMN label text NOT NULL DEFAULT 'Original device'
    CHECK(char_length(label) BETWEEN 1 AND 40);
CREATE INDEX career_active_keys ON career_keys(profile_id) WHERE revoked_at IS NULL;
INSERT INTO career_schema_version VALUES(3);
