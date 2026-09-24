INSERT INTO track_work_aliases (track_id, alias_key)
SELECT alias.track_id, alias.alias_key
FROM UNNEST($1::uuid[], $2::text[]) AS alias(track_id, alias_key)
ON CONFLICT (track_id, alias_key) DO NOTHING
