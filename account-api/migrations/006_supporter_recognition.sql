-- Payment alone never publishes a player's support. Existing preferences stay private.
ALTER TABLE portal.supporter_preferences
  ADD COLUMN recognition_visible boolean NOT NULL DEFAULT false;
CREATE INDEX supporter_recognition_profiles ON portal.supporter_preferences(profile_id)
  WHERE recognition_visible;
INSERT INTO portal.schema_version VALUES(6);
