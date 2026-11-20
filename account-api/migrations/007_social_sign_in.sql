-- Sign in with VK ID (VK, OK, Mail.ru are one VK ID app). A VK user is stored only
-- as an HMAC of its user id under OMOBA_SOCIAL_SECRET: no name, e-mail, avatar or
-- token is kept. A website sign-in never creates a game profile; an unlinked
-- identity waits until a game confirms the link with its own device key.
CREATE TABLE portal.social_identities (
    provider text NOT NULL CHECK(provider IN('vk')),
    subject_hash text NOT NULL CHECK(subject_hash ~ '^[0-9a-f]{64}$'),
    method text NOT NULL CHECK(method IN('vkid','ok_ru','mail_ru')),
    profile_id text REFERENCES public.career_profiles(profile_id),
    created_at bigint NOT NULL,
    last_login_at bigint NOT NULL,
    linked_at bigint,
    PRIMARY KEY(provider,subject_hash),
    CHECK((profile_id IS NULL)=(linked_at IS NULL))
);
-- One VK account per game profile; races resolve on this index.
CREATE UNIQUE INDEX social_identity_profile ON portal.social_identities(profile_id,provider)
    WHERE profile_id IS NOT NULL;
CREATE INDEX social_identity_unlinked ON portal.social_identities(last_login_at)
    WHERE profile_id IS NULL;
-- Browser-held proof of a just-verified, still unlinked identity (30 minutes).
-- The display name ("Иван П.") lives only here and in social_links, and expires with them.
CREATE TABLE portal.social_tickets (
    ticket_hash text PRIMARY KEY CHECK(ticket_hash ~ '^[0-9a-f]{64}$'),
    provider text NOT NULL,
    subject_hash text NOT NULL,
    display_name text NOT NULL CHECK(char_length(display_name) BETWEEN 1 AND 40),
    expires_at bigint NOT NULL,
    FOREIGN KEY(provider,subject_hash) REFERENCES portal.social_identities(provider,subject_hash)
        ON DELETE CASCADE
);
CREATE INDEX social_ticket_expiry ON portal.social_tickets(expires_at);
-- An unlinked identity offered to one pending native enrollment; the game links it
-- to its current profile with a signature of its active key (POST /v1/auth/social/link).
CREATE TABLE portal.social_links (
    enrollment_id text PRIMARY KEY REFERENCES portal.device_enrollments(enrollment_id)
        ON DELETE CASCADE,
    provider text NOT NULL,
    subject_hash text NOT NULL,
    display_name text NOT NULL CHECK(char_length(display_name) BETWEEN 1 AND 40),
    state text NOT NULL DEFAULT 'pending' CHECK(state IN('pending','linked')),
    created_at bigint NOT NULL,
    FOREIGN KEY(provider,subject_hash) REFERENCES portal.social_identities(provider,subject_hash)
        ON DELETE CASCADE
);
INSERT INTO portal.schema_version VALUES(7);
