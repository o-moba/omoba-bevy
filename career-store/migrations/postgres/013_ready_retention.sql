-- Bound maintenance by indexed deadlines/closed time. Never prune live ownership
-- or an unfinished match, even when its ready receipt is old and consumed.
CREATE INDEX career_ready_expiry ON career_ready_checks(deadline_ms,check_id)
    WHERE state IN ('forming','open','approved');
CREATE INDEX career_ready_retention ON career_ready_checks(updated_at,check_id)
    WHERE state IN ('cancelled','expired','consumed');

-- Runtime may invoke this narrow policy, not DELETE arbitrary consent records.
CREATE FUNCTION career_prune_closed_ready(batch_size INTEGER) RETURNS INTEGER
LANGUAGE plpgsql SECURITY DEFINER AS $$
DECLARE pruned INTEGER;
BEGIN
    IF batch_size IS NULL OR batch_size < 1 OR batch_size > 128 THEN
        RAISE EXCEPTION 'ready retention needs 1-128 checks per batch'
            USING ERRCODE = '22023';
    END IF;
    WITH candidates AS (
        SELECT r.check_id FROM career_ready_checks r
        WHERE r.state IN ('cancelled','expired','consumed')
          AND r.updated_at < statement_timestamp() - interval '7 days'
          AND NOT EXISTS (SELECT 1 FROM career_ready_claims c WHERE c.check_id=r.check_id)
          AND NOT EXISTS (SELECT 1 FROM career_matches m
                          WHERE m.result_id=r.check_id AND m.status <> 'settled')
        ORDER BY r.updated_at,r.check_id LIMIT batch_size
        FOR UPDATE OF r SKIP LOCKED
    ), removed AS (
        DELETE FROM career_ready_checks r USING candidates c
        WHERE r.check_id=c.check_id RETURNING r.check_id
    ) SELECT count(*)::integer INTO pruned FROM removed;
    RETURN pruned;
END $$;
DO $$
BEGIN
    EXECUTE format(
        'ALTER FUNCTION career_prune_closed_ready(integer) SET search_path = %s, pg_temp',
        (SELECT string_agg(quote_ident(name), ', ') FROM unnest(current_schemas(false)) AS name));
END $$;
REVOKE ALL ON FUNCTION career_prune_closed_ready(integer) FROM PUBLIC;
INSERT INTO career_schema_version VALUES(13);
