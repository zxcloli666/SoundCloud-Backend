WITH weak AS (
    SELECT credit.track_id,
           credit.artist_id,
           credit.role
    FROM track_artists AS credit
    JOIN tracks AS track
      ON track.id = credit.track_id
    JOIN artist_sc_accounts AS account
      ON account.artist_id = credit.artist_id
     AND account.sc_user_id = track.uploader_sc_user_id
    WHERE credit.evidence IN (
              'uploader_name',
              'title_heuristic',
              'ai_inference',
              'unattributed'
          )
      AND track.uploader_sc_user_id IS NOT NULL
      AND (account.verified OR account.source = 'mb_resolve')
    ORDER BY credit.track_id, credit.artist_id, credit.role
    LIMIT $1
    FOR UPDATE OF credit SKIP LOCKED
)
UPDATE track_artists AS credit
SET source = 'sc_verified',
    confidence = greatest(credit.confidence, 0.95),
    evidence = 'verified_account'
FROM weak
WHERE credit.track_id = weak.track_id
  AND credit.artist_id = weak.artist_id
  AND credit.role = weak.role
RETURNING credit.track_id
