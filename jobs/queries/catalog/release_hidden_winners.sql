WITH hidden AS (SELECT loser.id
                FROM tracks AS loser
                         JOIN tracks AS winner ON winner.id = loser.superseded_by
                WHERE loser.superseded_by IS NOT NULL
                  AND (winner.sharing IS DISTINCT FROM 'public'
                    OR winner.deleted_at IS NOT NULL
                    OR winner.superseded_by IS NOT NULL
                    OR loser.duration_ms IS NULL
                    OR winner.duration_ms IS NULL)
                LIMIT $1),
     released AS (
         UPDATE tracks
             SET superseded_by = NULL,
                 updated_at = now()
             FROM hidden
             WHERE tracks.id = hidden.id
             RETURNING tracks.id)
SELECT count(*) AS "released!"
FROM released
