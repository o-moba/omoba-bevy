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
GRANT SELECT,INSERT,UPDATE,DELETE ON portal.ekza_library_links TO :"portal_role";
GRANT SELECT,INSERT,UPDATE,DELETE ON portal.class_builds TO :"portal_role";
GRANT SELECT,INSERT,UPDATE,DELETE ON portal.device_enrollments,portal.recovery_codes,
  portal.supporter_nonces TO :"portal_role";
-- Sign in with VK ID (portal schema 7): keyed identity hashes, tickets, game links.
GRANT SELECT,INSERT,UPDATE,DELETE ON portal.social_identities,portal.social_tickets,
  portal.social_links TO :"portal_role";
GRANT EXECUTE ON FUNCTION portal.register_social_profile(text,text) TO :"portal_role";
GRANT SELECT,INSERT ON portal.supporter_events TO :"portal_role";
GRANT SELECT,INSERT,UPDATE ON portal.supporter_accounts,portal.supporter_grants,
  portal.supporter_preferences,portal.supporter_orders TO :"portal_role";
-- YooKassa donations (portal schema 9): attempts, refunds, the refund-scan cursor
-- and operator issues are billing audit, so no DELETE. Only the Account API
-- (this role) writes grants and their tier points.
GRANT SELECT,INSERT,UPDATE ON portal.supporter_yookassa_attempts,
  portal.supporter_yookassa_refund_scan,portal.supporter_yookassa_issues TO :"portal_role";
GRANT SELECT,INSERT ON portal.supporter_yookassa_refunds TO :"portal_role";
GRANT EXECUTE ON FUNCTION portal.supporter_points(text,bigint),
  portal.supporter_months(text) TO :"portal_role";
GRANT USAGE ON SCHEMA portal TO :"game_role";
GRANT SELECT ON portal.supporter_grants,portal.supporter_preferences TO :"game_role";
GRANT INSERT,UPDATE ON portal.supporter_preferences TO :"game_role";
-- The game reads the Hero tier sum (status, equip, the server's aura check).
GRANT EXECUTE ON FUNCTION portal.supporter_points(text,bigint) TO :"game_role";
GRANT SELECT ON career_schema_version TO :"game_role";
GRANT SELECT,INSERT,UPDATE,DELETE ON career_profiles,career_matches,
  career_participants,career_active_profiles,career_friendships,career_presence TO :"game_role";
GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO :"game_role";
-- Revoked-key tombstones must survive normal runtime operations. Remove legacy
-- broad grants when upgrading an existing game role, then grant only registration.
REVOKE INSERT,UPDATE,DELETE ON career_keys FROM :"game_role";
GRANT SELECT,INSERT(public_key,profile_id) ON career_keys TO :"game_role";
-- Admission limits (career schema 4, LR-16): the game counts profiles created for
-- new keys and drops counter rows after 48 hours. Only the owner-defined retention
-- function deletes unplayed, unlinked profiles and their keys.
GRANT SELECT ON career_admission_salt TO :"game_role";
GRANT SELECT,INSERT,DELETE ON career_profile_admissions TO :"game_role";
GRANT EXECUTE ON FUNCTION career_prune_unplayed_profiles(integer, integer) TO :"game_role";
-- Moderation (career schema 5, LR-15): game servers read sanctions and the chat
-- switch. Operators run `omoba-account-api sanction|rename|chat` with the portal
-- role: it applies and lifts sanctions (only the lift columns may change), writes
-- the append-only audit and switches chat. No runtime role may delete either.
GRANT SELECT ON career_sanctions,career_chat_switch TO :"game_role";
GRANT SELECT,INSERT,UPDATE(lifted_at,lifted_by,lift_reason) ON career_sanctions TO :"portal_role";
GRANT SELECT,INSERT ON career_moderation_events TO :"portal_role";
GRANT SELECT,UPDATE(enabled,changed_at,changed_by) ON career_chat_switch TO :"portal_role";
GRANT USAGE ON SEQUENCE career_sanctions_sanction_id_seq,career_moderation_events_seq_seq
  TO :"portal_role";
-- Neither runtime role receives schema CREATE, ownership, or migration privileges.
-- In a legacy database, explicitly remove public schema CREATE from PUBLIC before
-- deployment. Fresh PostgreSQL 15+ databases already use the owner-only default.
