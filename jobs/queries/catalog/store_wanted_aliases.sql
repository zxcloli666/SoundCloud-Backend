INSERT INTO wanted_track_work_aliases (wanted_track_id, alias_key)
SELECT alias.wanted_track_id, alias.alias_key
FROM UNNEST($1::uuid[], $2::text[]) AS alias(wanted_track_id, alias_key)
ON CONFLICT (wanted_track_id, alias_key) DO NOTHING
