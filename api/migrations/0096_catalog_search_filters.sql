CREATE INDEX tracks_public_genre_search_idx ON tracks (LOWER(genre))
WHERE sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL;

CREATE INDEX tracks_public_tags_search_idx ON tracks USING gin (tags)
WHERE sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL;
