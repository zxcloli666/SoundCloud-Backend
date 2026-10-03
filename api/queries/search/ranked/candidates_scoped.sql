WITH scoped AS (
    SELECT id
    FROM tracks
    WHERE primary_artist_id = ANY ($1)
    UNION
    SELECT id
    FROM tracks
    WHERE uploader_sc_user_id = ANY ($2)
    UNION
    SELECT track.id
    FROM artist_sc_accounts AS account
    JOIN tracks AS track ON track.uploader_sc_user_id = account.sc_user_id
    WHERE account.artist_id = ANY ($1)
    UNION
    SELECT track_id
    FROM track_artists
    WHERE role IN ('primary', 'featured')
      AND artist_id = ANY ($1)
)
SELECT track.id AS "id!",
       track.title AS "title!",
       track.uploader_username,
       track.metadata_artist,
       track.play_count_sc,
       track.sc_metadata ->> 'access' AS access,
       EXISTS (SELECT 1
               FROM artist_sc_accounts AS account
               WHERE account.artist_id = track.primary_artist_id
                 AND account.sc_user_id = track.uploader_sc_user_id) AS "linked!"
FROM scoped
JOIN tracks AS track ON track.id = scoped.id
WHERE track.sharing = 'public'
  AND track.deleted_at IS NULL
  AND track.superseded_by IS NULL
  AND track.title_normalized LIKE ALL ($3)
ORDER BY track.play_count_sc DESC NULLS LAST, track.id DESC
LIMIT 150
