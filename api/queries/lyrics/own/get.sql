SELECT plain_text, synced_lrc, origin, lrclib_id
FROM user_lyrics
WHERE soundcloud_user_id = $1
  AND sc_track_id = $2
