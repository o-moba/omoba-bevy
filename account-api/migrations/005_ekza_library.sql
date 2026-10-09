-- A project-scoped library grant, not a second login identity or SSO mapping.
CREATE TABLE portal.ekza_library_links (
    profile_id text PRIMARY KEY REFERENCES public.career_profiles(profile_id),
    registry_origin text NOT NULL,
    phase text NOT NULL CHECK (phase IN ('pending','connected')),
    sealed_secret bytea NOT NULL CHECK (octet_length(sealed_secret) BETWEEN 60 AND 512),
    username text,
    user_code text,
    verification_url text,
    expires_at text NOT NULL,
    pending_until bigint NOT NULL,
    next_poll bigint NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK ((phase='connected' AND username IS NOT NULL AND user_code IS NULL AND verification_url IS NULL)
        OR (phase='pending' AND username IS NULL AND user_code IS NOT NULL AND verification_url IS NOT NULL))
);
REVOKE ALL ON portal.ekza_library_links FROM PUBLIC;
INSERT INTO portal.schema_version VALUES(5);
