-- Run as migration owner using psql -v portal_role=... -v game_role=... -f grants.sql.
-- Roles must already exist. No passwords, role creation or production defaults here.
GRANT USAGE ON SCHEMA public TO :"portal_role", :"game_role";
GRANT SELECT ON career_schema_version,career_profiles,career_keys,career_matches,
  career_participants,career_active_profiles,career_friendships,career_presence TO :"portal_role";
GRANT UPDATE(nickname) ON career_profiles TO :"portal_role";
-- Native enrollment may add a key or revoke it, never move an existing key.
GRANT INSERT(public_key,profile_id,label),UPDATE(revoked_at) ON career_keys TO :"portal_role";
GRANT INSERT,UPDATE,DELETE ON career_friendships TO :"portal_role";
GRANT USAGE ON SCHEMA portal TO :"portal_role";
GRANT SELECT ON portal.schema_version TO :"portal_role";
GRANT SELECT,INSERT,UPDATE,DELETE ON portal.web_pairings,portal.web_sessions,
  portal.profile_settings,portal.operation_receipts,portal.rate_limits,
  portal.audit_events,portal.projected_results,portal.player_match_facts TO :"portal_role";
GRANT USAGE ON ALL SEQUENCES IN SCHEMA portal TO :"portal_role";
GRANT SELECT,INSERT,UPDATE,DELETE ON portal.device_enrollments,portal.recovery_codes,
  portal.supporter_nonces TO :"portal_role";
GRANT SELECT,INSERT ON portal.supporter_events TO :"portal_role";
GRANT SELECT,INSERT,UPDATE ON portal.supporter_accounts,portal.supporter_grants,
  portal.supporter_preferences,portal.supporter_orders TO :"portal_role";
GRANT USAGE ON SCHEMA portal TO :"game_role";
GRANT SELECT ON portal.supporter_grants,portal.supporter_preferences TO :"game_role";
GRANT INSERT,UPDATE ON portal.supporter_preferences TO :"game_role";
GRANT SELECT ON career_schema_version TO :"game_role";
GRANT SELECT,INSERT,UPDATE,DELETE ON career_profiles,career_matches,
  career_participants,career_active_profiles,career_friendships,career_presence TO :"game_role";
GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO :"game_role";
-- Revoked-key tombstones must survive normal runtime operations. Remove legacy
-- broad grants when upgrading an existing game role, then grant only registration.
REVOKE INSERT,UPDATE,DELETE ON career_keys FROM :"game_role";
GRANT SELECT,INSERT(public_key,profile_id) ON career_keys TO :"game_role";
-- Neither runtime role receives schema CREATE, ownership, or migration privileges.
-- In a legacy database, explicitly remove public schema CREATE from PUBLIC before
-- deployment. Fresh PostgreSQL 15+ databases already use the owner-only default.
