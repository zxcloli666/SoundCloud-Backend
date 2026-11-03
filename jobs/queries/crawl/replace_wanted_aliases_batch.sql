WITH batch AS (
    SELECT UNNEST($1::uuid[]) AS wanted_track_id
), input AS (
    SELECT alias.wanted_track_id, alias.alias_key
    FROM UNNEST($2::uuid[], $3::text[]) AS alias(wanted_track_id, alias_key)
), cleared AS (
    DELETE FROM wanted_track_work_aliases AS stored
        USING batch
        WHERE stored.wanted_track_id = batch.wanted_track_id
            AND NOT EXISTS (SELECT 1
                            FROM input
                            WHERE input.wanted_track_id = stored.wanted_track_id
                              AND input.alias_key = stored.alias_key)
        RETURNING stored.wanted_track_id
)
INSERT INTO wanted_track_work_aliases (wanted_track_id, alias_key)
SELECT wanted_track_id, alias_key
FROM input
ON CONFLICT (wanted_track_id, alias_key) DO NOTHING
