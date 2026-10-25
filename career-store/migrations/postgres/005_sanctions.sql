-- Moderation (LR-15): sanctions on profiles, an append-only moderation audit and
-- the database-wide chat switch. Executed transactionally under the adapter's
-- migration advisory lock.

-- One row per sanction an operator applied (`omoba-account-api sanction add`).
-- A sanction ends at `expires_at` (NULL: until lifted) or when it is lifted; a
-- lifted row keeps who lifted it and why. Rows are never deleted by a runtime
-- role, and a sanctioned profile is never pruned as unplayed: this foreign key
-- keeps it, so a ban on a throwaway key holds.
CREATE TABLE career_sanctions (
    sanction_id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES career_profiles(profile_id),
    kind TEXT NOT NULL CHECK (kind IN
        ('chat_mute','matchmaking_ban','ranked_ban','account_ban','name_reset')),
    reason TEXT NOT NULL CHECK (char_length(reason) BETWEEN 1 AND 500),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ CHECK (expires_at IS NULL OR expires_at > created_at),
    created_by TEXT NOT NULL CHECK (char_length(created_by) BETWEEN 1 AND 64),
    lifted_at TIMESTAMPTZ,
    lifted_by TEXT CHECK (char_length(lifted_by) BETWEEN 1 AND 64),
    lift_reason TEXT CHECK (char_length(lift_reason) BETWEEN 1 AND 500),
    CHECK ((lifted_at IS NULL) = (lifted_by IS NULL)
        AND (lifted_at IS NULL) = (lift_reason IS NULL))
);
CREATE INDEX career_sanctions_live ON career_sanctions(profile_id) WHERE lifted_at IS NULL;
CREATE INDEX career_sanctions_profile ON career_sanctions(profile_id, sanction_id);

-- Every moderation action writes one row here in the same transaction: applying
-- or lifting a sanction (a name reset includes the old and the new name) and
-- switching chat. No foreign key: the trail outlives the profile. Rows cannot be
-- changed, and cannot be deleted before they are a year old (the trigger below);
-- nothing deletes them automatically.
CREATE TABLE career_moderation_events (
    seq BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    actor TEXT NOT NULL CHECK (char_length(actor) BETWEEN 1 AND 64),
    action TEXT NOT NULL CHECK (action IN ('sanction_added','sanction_lifted','chat_switched')),
    profile_id TEXT CHECK (profile_id ~ '^[0-9a-f]{64}$'),
    sanction_id BIGINT,
    detail JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(detail) = 'object'),
    CHECK ((action = 'chat_switched') = (profile_id IS NULL))
);
CREATE INDEX career_moderation_events_profile ON career_moderation_events(profile_id, seq);

CREATE FUNCTION career_guard_moderation_event() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE' THEN
        RAISE EXCEPTION 'moderation events are append-only' USING ERRCODE = '23514';
    END IF;
    IF OLD.created_at > clock_timestamp() - interval '1 year' THEN
        RAISE EXCEPTION 'moderation events are kept for at least a year' USING ERRCODE = '23514';
    END IF;
    RETURN OLD;
END $$;
CREATE TRIGGER career_moderation_event_guard BEFORE UPDATE OR DELETE ON career_moderation_events
    FOR EACH ROW EXECUTE FUNCTION career_guard_moderation_event();

-- The database-wide chat switch (`omoba-account-api chat on|off`). Every game
-- server polls it; `OMOBA_CHAT=off` on a server keeps chat off there whatever
-- this says. Reactions are never switched off.
CREATE TABLE career_chat_switch (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    changed_by TEXT NOT NULL DEFAULT 'migration' CHECK (char_length(changed_by) BETWEEN 1 AND 64)
);
INSERT INTO career_chat_switch DEFAULT VALUES;
INSERT INTO career_schema_version VALUES(5);
