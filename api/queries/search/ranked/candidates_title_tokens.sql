WITH hits AS MATERIALIZED (
    SELECT id,
           title,
           uploader_username,
           metadata_artist,
           play_count_sc,
           sc_metadata ->> 'access' AS access,
           primary_artist_id,
           uploader_sc_user_id
    FROM tracks
    WHERE sharing = 'public'
      AND deleted_at IS NULL
      AND superseded_by IS NULL
      AND title_normalized LIKE $1
      AND ($2::text IS NULL OR title_normalized LIKE $2)
      AND ($3::text IS NULL OR title_normalized LIKE $3)
      AND ($4::text IS NULL OR title_normalized LIKE $4)
    LIMIT 1000
)
SELECT hits.id AS "id!",
       hits.title AS "title!",
       hits.uploader_username,
       hits.metadata_artist,
       hits.play_count_sc,
       hits.access,
       EXISTS (SELECT 1
               FROM artist_sc_accounts AS account
               WHERE account.artist_id = hits.primary_artist_id
                 AND account.sc_user_id = hits.uploader_sc_user_id) AS "linked!"
FROM hits
ORDER BY hits.play_count_sc DESC NULLS LAST, hits.id DESC
LIMIT 300
