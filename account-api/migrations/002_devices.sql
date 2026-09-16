-- Expanded signed consent must not upgrade existing limited browser sessions.
-- Default 1 prevents an old API replica from creating newly privileged sessions.
ALTER TABLE portal.web_sessions ADD COLUMN authorization_version smallint NOT NULL DEFAULT 1
    CHECK(authorization_version IN(1,2));
ALTER TABLE portal.web_pairings ADD COLUMN authorization_version smallint NOT NULL DEFAULT 1
    CHECK(authorization_version IN(1,2));
UPDATE portal.web_sessions SET revoked=true;
UPDATE portal.web_pairings SET state='cancelled',delivery=NULL,delivery_until=NULL;
CREATE TABLE portal.device_enrollments (
    enrollment_id text PRIMARY KEY CHECK(enrollment_id ~ '^[0-9a-f]{64}$'),
    public_key text NOT NULL CHECK(public_key ~ '^[0-9a-f]{64}$'),
    code_hash text NOT NULL UNIQUE,
    code_delivery bytea NOT NULL,
    label text NOT NULL CHECK(char_length(label) BETWEEN 1 AND 40),
    origin text NOT NULL,
    expires_at bigint NOT NULL,
    state text NOT NULL DEFAULT 'pending' CHECK(state IN('pending','approved','consumed')),
    profile_id text REFERENCES public.career_profiles(profile_id),
    approved_at bigint,
    CHECK((state='pending' AND profile_id IS NULL) OR (state!='pending' AND profile_id IS NOT NULL))
);
CREATE INDEX device_enrollment_expiry ON portal.device_enrollments(expires_at);
CREATE TABLE portal.recovery_codes (
    code_hash text PRIMARY KEY,
    profile_id text NOT NULL REFERENCES public.career_profiles(profile_id),
    created_at bigint NOT NULL,
    consumed_at bigint,
    enrollment_id text REFERENCES portal.device_enrollments(enrollment_id)
);
CREATE INDEX recovery_profile ON portal.recovery_codes(profile_id);
INSERT INTO portal.schema_version VALUES(2);
