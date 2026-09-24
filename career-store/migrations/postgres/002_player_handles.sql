-- Add human-readable addresses without changing profile IDs or match receipts.
LOCK TABLE career_profiles IN ACCESS EXCLUSIVE MODE;
ALTER TABLE career_profiles DROP CONSTRAINT career_profiles_nickname_check;
CREATE UNIQUE INDEX career_handle_migrating ON career_profiles(lower(nickname COLLATE "und-x-icu"))
WHERE nickname ~ '^[^#]+#[0-9]{4}$';
CREATE FUNCTION career_assign_handle() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE base text; candidate text; start_tag integer; available text; n integer;
BEGIN
  IF TG_OP='UPDATE' AND NEW.nickname=OLD.nickname AND NEW.nickname ~ '^[^#]+#[0-9]{4}$' THEN RETURN NEW; END IF;
  IF position('#' IN NEW.nickname)>0 THEN RETURN NEW; END IF;
  base := NEW.nickname;
  IF base='Player' THEN
    base := (ARRAY['Amber','Brave','Calm','Cloud','Ember','Frost','Jade','Lunar','Moss','River','Swift','Wild'])[1+floor(random()*12)::integer]
         || (ARRAY['Badger','Bear','Crane','Deer','Fox','Hawk','Lynx','Otter','Owl','Panda','Tiger','Wolf'])[1+floor(random()*12)::integer];
  END IF;
  IF TG_OP='UPDATE' AND OLD.nickname ~ '^[^#]+#[0-9]{4}$' THEN
    NEW.nickname := base || '#' || right(OLD.nickname,4);
    RETURN NEW;
  END IF;
  -- Bound lock cardinality for bulk imports; migration updates already hold a table lock.
  IF TG_OP='INSERT' THEN
    PERFORM pg_advisory_xact_lock(721946105,(hashtextextended(lower(base COLLATE "und-x-icu"),0)&1023)::integer);
  END IF;
  start_tag := floor(random()*10000)::integer;
  FOR n IN 0..9999 LOOP
    candidate := base || '#' || lpad(((start_tag+n)%10000)::text,4,'0');
    IF NOT EXISTS(SELECT 1 FROM career_profiles p WHERE p.nickname ~ '^[^#]+#[0-9]{4}$' AND lower(p.nickname COLLATE "und-x-icu")=lower(candidate COLLATE "und-x-icu")) THEN
      available := candidate; EXIT;
    END IF;
  END LOOP;
  IF available IS NULL THEN RAISE EXCEPTION 'This nickname has no free tags; choose another name' USING ERRCODE='23505',CONSTRAINT='career_player_handle'; END IF;
  NEW.nickname := available;
  RETURN NEW;
END $$;
CREATE TRIGGER career_handle_assignment BEFORE INSERT OR UPDATE OF nickname ON career_profiles
FOR EACH ROW EXECUTE FUNCTION career_assign_handle();
UPDATE career_profiles SET nickname=nickname;
CREATE UNIQUE INDEX career_player_handle ON career_profiles(lower(nickname COLLATE "und-x-icu"));
DROP INDEX career_handle_migrating;
ALTER TABLE career_profiles ADD CONSTRAINT career_profiles_nickname_check
CHECK (nickname ~ '^[^#]{1,20}#[0-9]{4}$');
INSERT INTO career_schema_version VALUES(2);
