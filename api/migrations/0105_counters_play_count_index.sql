CREATE INDEX IF NOT EXISTS sc_track_counters_play_count_idx
    ON sc_track_counters (play_count DESC NULLS LAST, sc_track_id);
