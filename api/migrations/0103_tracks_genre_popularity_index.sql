CREATE INDEX IF NOT EXISTS tracks_public_genre_popular_idx
    ON tracks (lower(genre), play_count_sc DESC NULLS LAST, sc_synced_at DESC, id DESC)
    WHERE sharing = 'public'
      AND deleted_at IS NULL
      AND superseded_by IS NULL;
