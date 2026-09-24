UPDATE catalog_merge_state
SET cursor_artist        = $1,
    cursor_recording_key = $2,
    groups_merged        = groups_merged + $3,
    tracks_superseded    = tracks_superseded + $4,
    updated_at           = now()
WHERE singleton
