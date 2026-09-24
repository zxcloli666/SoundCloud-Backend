SELECT it.sc_track_id
FROM tracks it
         JOIN sc_track_counters c ON c.sc_track_id = it.sc_track_id
WHERE it.sharing = 'public'
  AND it.superseded_by IS NULL
ORDER BY c.play_count DESC NULLS LAST LIMIT $1
