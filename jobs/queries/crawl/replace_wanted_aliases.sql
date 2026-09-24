WITH cleared AS (
    DELETE FROM wanted_track_work_aliases
        WHERE wanted_track_id = $1
        RETURNING wanted_track_id)
INSERT
INTO wanted_track_work_aliases (wanted_track_id, alias_key)
SELECT $1, alias_key
FROM UNNEST($2::text[]) AS alias(alias_key)
ON CONFLICT (wanted_track_id, alias_key) DO NOTHING
