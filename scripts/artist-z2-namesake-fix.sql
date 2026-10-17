\set ON_ERROR_STOP on
\if :{?apply}
\else
\set apply off
\endif

BEGIN;

SELECT account.sc_user_id, account.role, account.source, account.verified
FROM artist_sc_accounts AS account
WHERE account.artist_id = '6cbf3702-d84e-4d0a-a713-e5cec09c7bfc'
ORDER BY account.sc_user_id;

SELECT artist.sc_user_id,
       artist.avatar_url,
       (SELECT replace(track.uploader_avatar_url, '-large.', '-t500x500.')
        FROM tracks AS track
        WHERE track.uploader_sc_user_id = '1211580634'
          AND track.uploader_avatar_url LIKE '%/avatars-%'
        ORDER BY track.updated_at DESC
        LIMIT 1) AS latest_home_upload_avatar
FROM artists AS artist
WHERE artist.id = '6cbf3702-d84e-4d0a-a713-e5cec09c7bfc';

CREATE TEMP TABLE uploader_credits ON COMMIT DROP AS
SELECT credit.track_id,
       track.uploader_sc_user_id,
       track.uploader_username,
       track.title,
       credit.source,
       credit.evidence,
       credit.evidence IN ('uploader_name', 'title_heuristic', 'ai_inference',
                           'metadata_field', 'unattributed') AS name_only,
       EXISTS (SELECT 1
               FROM track_artists AS other
               WHERE other.track_id = credit.track_id
                 AND other.artist_id = credit.artist_id
                 AND other.role <> 'primary')               AS keeps_other_role
FROM track_artists AS credit
         JOIN tracks AS track ON track.id = credit.track_id
WHERE credit.artist_id = '6cbf3702-d84e-4d0a-a713-e5cec09c7bfc'
  AND credit.role = 'primary'
  AND track.uploader_sc_user_id IN ('972358267', '1281291292', '262191304', '1384256286',
                                    '1280256208', '1072135747', '978497614', '1270026898');

SELECT credit.uploader_sc_user_id,
       credit.uploader_username,
       credit.title,
       credit.source,
       credit.evidence
FROM uploader_credits AS credit
WHERE NOT credit.name_only
ORDER BY credit.uploader_sc_user_id, credit.title;

SELECT credit.uploader_sc_user_id,
       credit.uploader_username,
       credit.title,
       credit.evidence,
       credit.keeps_other_role
FROM uploader_credits AS credit
WHERE credit.name_only
ORDER BY credit.uploader_sc_user_id, credit.title;

DELETE
FROM artist_sc_accounts
WHERE artist_id = '6cbf3702-d84e-4d0a-a713-e5cec09c7bfc'
  AND sc_user_id IN ('972358267', '1281291292', '262191304', '1384256286',
                     '1280256208', '1072135747', '978497614', '1270026898')
  AND NOT verified
  AND source IN ('auto_match', 'reupload_pattern');

UPDATE artists
SET sc_user_id = '1211580634',
    avatar_url = 'https://i1.sndcdn.com/avatars-0nqZTr2cOae7fdHx-z56c8Q-t500x500.jpg',
    updated_at = now()
WHERE id = '6cbf3702-d84e-4d0a-a713-e5cec09c7bfc';

INSERT INTO track_artist_blocks (track_id, artist_id, note)
SELECT credit.track_id, '6cbf3702-d84e-4d0a-a713-e5cec09c7bfc', 'GH-BI#6 namesake upload'
FROM uploader_credits AS credit
WHERE credit.name_only
  AND NOT credit.keeps_other_role
ON CONFLICT (track_id, artist_id) DO NOTHING;

DELETE
FROM track_artists AS credit
    USING uploader_credits AS namesake
WHERE namesake.name_only
  AND credit.track_id = namesake.track_id
  AND credit.artist_id = '6cbf3702-d84e-4d0a-a713-e5cec09c7bfc'
  AND credit.role = 'primary';

UPDATE tracks AS track
SET primary_artist_id = (SELECT credit.artist_id
                         FROM track_artists AS credit
                         WHERE credit.track_id = track.id
                           AND credit.role = 'primary'
                         ORDER BY credit.position
                         LIMIT 1)
FROM uploader_credits AS namesake
WHERE namesake.name_only
  AND track.id = namesake.track_id
  AND track.primary_artist_id = '6cbf3702-d84e-4d0a-a713-e5cec09c7bfc';

UPDATE tracks AS track
SET enrich_state       = 'pending',
    enrich_source      = NULL,
    enrich_confidence  = NULL,
    enrich_error       = NULL,
    enrich_attempts    = 0,
    enrich_next_run_at = now() + random() * interval '15 minutes',
    updated_at         = now()
FROM uploader_credits AS namesake
WHERE namesake.name_only
  AND track.id = namesake.track_id;

SELECT account.sc_user_id, account.role, account.source, account.verified
FROM artist_sc_accounts AS account
WHERE account.artist_id = '6cbf3702-d84e-4d0a-a713-e5cec09c7bfc'
ORDER BY account.sc_user_id;

SELECT artist.sc_user_id, artist.avatar_url
FROM artists AS artist
WHERE artist.id = '6cbf3702-d84e-4d0a-a713-e5cec09c7bfc';

SELECT credit.role, count(*) AS tracks, count(DISTINCT track.uploader_sc_user_id) AS uploaders
FROM track_artists AS credit
         JOIN tracks AS track ON track.id = credit.track_id
WHERE credit.artist_id = '6cbf3702-d84e-4d0a-a713-e5cec09c7bfc'
GROUP BY credit.role
ORDER BY credit.role;

\if :apply
COMMIT;
\else
ROLLBACK;
\endif
