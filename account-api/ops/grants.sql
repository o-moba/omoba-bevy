-- Run as migration owner using psql -v portal_role=... -v game_role=... -f grants.sql.
-- Roles must already exist. No passwords, role creation or production defaults here.
GRANT USAGE ON SCHEMA public TO :"portal_role", :"game_role";
GRANT SELECT ON career_schema_version,career_profiles,career_keys,career_matches,
  career_participants,career_active_profiles,career_friendships,career_presence TO :"portal_role";
GRANT UPDATE(nickname) ON career_profiles TO :"portal_role";
GRANT INSERT,UPDATE,DELETE ON career_friendships TO :"portal_role";
GRANT USAGE ON SCHEMA portal TO :"portal_role";
GRANT SELECT ON portal.schema_version TO :"portal_role";
GRANT SELECT,INSERT,UPDATE,DELETE ON portal.web_pairings,portal.web_sessions,
  portal.profile_settings,portal.operation_receipts,portal.rate_limits,
  portal.audit_events,portal.projected_results,portal.player_match_facts TO :"portal_role";
GRANT USAGE ON ALL SEQUENCES IN SCHEMA portal TO :"portal_role";
GRANT SELECT ON career_schema_version TO :"game_role";
GRANT SELECT,INSERT,UPDATE,DELETE ON career_profiles,career_keys,career_matches,
  career_participants,career_active_profiles,career_friendships,career_presence TO :"game_role";
GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO :"game_role";
-- Neither runtime role receives schema CREATE, ownership, or migration privileges.
-- In a legacy database, explicitly remove public schema CREATE from PUBLIC before
-- deployment. Fresh PostgreSQL 15+ databases already use the owner-only default.
