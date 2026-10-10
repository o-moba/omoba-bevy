-- Private named gameplay preferences. Saving a draft never grants match admission.
CREATE TABLE portal.class_builds (
    id text PRIMARY KEY CHECK (id ~ '^[0-9a-f]{64}$'),
    profile_id text NOT NULL REFERENCES public.career_profiles(profile_id),
    document jsonb NOT NULL CHECK (
        jsonb_typeof(document) = 'object'
        AND octet_length(document::text) <= 8192
        AND document ?& ARRAY['schema','name','description','recipe']
        AND document->>'schema' = 'omoba.class-build.v1'
    ),
    version bigint NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX class_builds_owner ON portal.class_builds(profile_id, updated_at DESC, id);
REVOKE ALL ON portal.class_builds FROM PUBLIC;
INSERT INTO portal.schema_version VALUES(4);
