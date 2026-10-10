WITH weak AS (
    SELECT credit.track_id,
           credit.artist_id,
           credit.role
    FROM artist_sc_accounts AS account
    JOIN tracks AS track
      ON track.uploader_sc_user_id = account.sc_user_id
    JOIN track_artists AS credit
      ON credit.track_id = track.id
     AND credit.artist_id = account.artist_id
    WHERE (account.verified OR account.source = 'mb_resolve')
      AND credit.evidence IN (
              'uploader_name',
              'title_heuristic',
              'ai_inference',
              'unattributed'
          )
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
