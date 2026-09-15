-- Synthetic benchmark data. Run ONLY in a dedicated disposable test database.
-- 100,000 settled results, 1,000,000 human facts, 10,000 profiles.
\set ON_ERROR_STOP on
BEGIN;
DO $$ BEGIN IF inet_server_addr() IS DISTINCT FROM '127.0.0.1'::inet OR inet_server_port() IS DISTINCT FROM 55581 THEN RAISE EXCEPTION 'Local disposable test database only'; END IF; END $$;
CREATE TEMP TABLE load_profiles AS SELECT n,md5('portal-benchmark-v2-'||n)||md5('profile-v2-'||n) AS id FROM generate_series(0,9999) n;
CREATE UNIQUE INDEX ON load_profiles(n);
ANALYZE load_profiles;
INSERT INTO career_profiles(profile_id,nickname) SELECT id,'Benchmark-'||n FROM load_profiles ON CONFLICT DO NOTHING;
CREATE TEMP TABLE load_results AS
SELECT n,'portal-load-v2-'||lpad(n::text,8,'0') AS id,
jsonb_build_object('result_id','portal-load-v2-'||lpad(n::text,8,'0'),'server_epoch',1,'match_id',n,
'started_at_ms',floor(extract(epoch FROM clock_timestamp())*1000)-600000-r.n*1000,
'ended_at_ms',floor(extract(epoch FROM clock_timestamp())*1000)-r.n*1000,'duration_ms',600000,
'map_profile','verdant','ruleset','verdant-default-v1','outcome','completed','winner','green','rated',true,'unrated_reason',NULL,'saved',true,
'participants',(SELECT jsonb_agg(jsonb_build_object('player_id',seat+1,'profile_id',p.id,'nickname','Benchmark-'||p.n,'team',CASE WHEN seat<5 THEN 'green' ELSE 'blue' END,'hero_class','ranger','character','archer','avatar',NULL,'sprite_character',NULL,'stats',jsonb_build_object('kills',3,'deaths',2,'assists',4,'damage_to_heroes',1200,'damage_to_structures',80,'damage_to_creeps',900,'damage_taken',400,'minion_last_hits',12,'jungle_last_hits',3,'structures_destroyed',1,'final_level',8),'disconnected',false,'rating',jsonb_build_object('before',1000,'after',1016,'delta',16),'progression_xp_gained',120) ORDER BY seat) FROM generate_series(0,9) seat JOIN load_profiles p ON p.n=(r.n*10+seat)%10000)) AS result
FROM generate_series(1,100000) AS r(n);
INSERT INTO career_matches(result_id,server_epoch,match_id,owner_id,lease_until,allocation,checkpoint)
SELECT id,'1',n::text,'portal-benchmark',clock_timestamp(),result,result FROM load_results ON CONFLICT(result_id) DO NOTHING;
INSERT INTO career_participants(result_id,seat,player_id,profile_id)
SELECT r.id,(p.ordinality-1)::smallint,p.value->>'player_id',p.value->>'profile_id' FROM load_results r JOIN career_matches m ON m.result_id=r.id CROSS JOIN LATERAL jsonb_array_elements(r.result->'participants') WITH ORDINALITY p WHERE m.status='running' ON CONFLICT DO NOTHING;
UPDATE career_matches m SET status='settled',intent=r.result,result=r.result,settled_at=clock_timestamp() FROM load_results r WHERE m.result_id=r.id AND m.status='running';
INSERT INTO portal.player_match_facts(result_id,player_id,profile_id,ended_at,duration_ms,hero_class,outcome,rated,won,kills,deaths,assists,damage_to_heroes,minion_last_hits,jungle_last_hits,rating_before,rating_after,rating_delta)
SELECT r.id,p->>'player_id',p->>'profile_id',to_timestamp((r.result->>'ended_at_ms')::double precision/1000),600000,'ranger','completed',true,p->>'team'='green',3,2,4,1200,12,3,1000,1016,16 FROM load_results r CROSS JOIN LATERAL jsonb_array_elements(r.result->'participants') p ON CONFLICT DO NOTHING;
INSERT INTO portal.projected_results(result_id,version,source_hash) SELECT id,1,'synthetic-benchmark' FROM load_results ON CONFLICT DO NOTHING;
COMMIT;
ANALYZE career_matches; ANALYZE career_participants; ANALYZE portal.player_match_facts; ANALYZE portal.projected_results;
SELECT count(*) AS load_results FROM career_matches WHERE result_id LIKE 'portal-load-v2-%';
SELECT count(*) AS load_facts FROM portal.player_match_facts WHERE result_id LIKE 'portal-load-v2-%';
