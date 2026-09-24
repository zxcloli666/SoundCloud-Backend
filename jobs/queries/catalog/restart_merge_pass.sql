UPDATE catalog_merge_state
SET cursor_artist        = NULL,
    cursor_recording_key = NULL,
    passes_completed     = passes_completed + 1,
    updated_at           = now()
WHERE singleton
