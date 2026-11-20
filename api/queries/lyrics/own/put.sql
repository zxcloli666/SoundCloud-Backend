INSERT INTO user_lyrics (soundcloud_user_id, sc_track_id, plain_text, synced_lrc, origin, lrclib_id)
SELECT $1, $2, NULLIF($3::text, ''), NULLIF($4::text, ''), $5, NULLIF($6::int8, 0)
WHERE EXISTS (
        SELECT 1
        FROM user_lyrics
        WHERE soundcloud_user_id = $1
          AND sc_track_id = $2
    )
   OR (
        SELECT count(*)
        FROM (
            SELECT 1
            FROM user_lyrics
            WHERE soundcloud_user_id = $1
            LIMIT $7
        ) AS mine
    ) < $7
ON CONFLICT (soundcloud_user_id, sc_track_id) DO UPDATE
SET plain_text = EXCLUDED.plain_text,
    synced_lrc = EXCLUDED.synced_lrc,
    origin = EXCLUDED.origin,
    lrclib_id = EXCLUDED.lrclib_id,
    updated_at = now()
