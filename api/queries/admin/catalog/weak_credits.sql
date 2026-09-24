SELECT credit.track_id,
       credit.artist_id,
       credit.role,
       credit.source,
       credit.confidence,
       credit.evidence,
       track.sc_track_id,
       track.title AS track_title,
       track.uploader_username,
       track.uploader_sc_user_id,
       artist.name AS artist_name,
       artist.mb_artist_id,
       artist.genius_artist_id,
       EXISTS (
           SELECT 1
           FROM artist_sc_accounts AS account
           WHERE account.artist_id = credit.artist_id
             AND (account.verified OR account.source = 'mb_resolve')
       ) AS "artist_has_identity!",
       (track.primary_artist_id = credit.artist_id) AS "is_primary!"
FROM track_artists AS credit
JOIN tracks AS track
  ON track.id = credit.track_id
JOIN artists AS artist
  ON artist.id = credit.artist_id
WHERE credit.evidence IN (
          'uploader_name',
          'title_heuristic',
          'ai_inference',
          'unattributed'
      )
  AND ($1::text IS NULL OR credit.evidence = $1)
ORDER BY credit.confidence, credit.track_id, credit.artist_id, credit.role
LIMIT $2 OFFSET $3
