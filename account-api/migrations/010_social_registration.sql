-- Website registration may create only a fresh profile with database defaults.
-- The portal runtime keeps no general INSERT/stat-update authority on profiles.
CREATE FUNCTION portal.register_social_profile(identity_provider text, identity_subject text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public, portal, pg_temp AS $$
DECLARE
    linked_profile text;
BEGIN
    IF identity_provider IS DISTINCT FROM 'vk' OR identity_subject IS NULL
       OR identity_subject !~ '^[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'invalid social identity' USING ERRCODE = '22023';
    END IF;
    SELECT profile_id INTO linked_profile FROM portal.social_identities
        WHERE provider = identity_provider AND subject_hash = identity_subject FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'verified social identity required' USING ERRCODE = '22023';
    END IF;
    IF linked_profile IS NOT NULL THEN
        RETURN linked_profile;
    END IF;
    linked_profile := replace(gen_random_uuid()::text, '-', '')
                   || replace(gen_random_uuid()::text, '-', '');
    -- No caller controls the profile id, nickname, progression or rating.
    INSERT INTO public.career_profiles(profile_id, nickname) VALUES(linked_profile, 'Player');
    UPDATE portal.social_identities
        SET profile_id = linked_profile, linked_at = floor(extract(epoch FROM clock_timestamp()))::bigint
        WHERE provider = identity_provider AND subject_hash = identity_subject;
    RETURN linked_profile;
END $$;
REVOKE ALL ON FUNCTION portal.register_social_profile(text, text) FROM PUBLIC;
INSERT INTO portal.schema_version VALUES(10);
