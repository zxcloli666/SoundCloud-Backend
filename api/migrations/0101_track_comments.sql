ALTER TABLE catalog_collection_sync
    DROP CONSTRAINT catalog_collection_sync_collection_check,
    DROP CONSTRAINT catalog_collection_public_scope;
ALTER TABLE catalog_collection_sync
    ADD CONSTRAINT catalog_collection_sync_collection_check CHECK (
        collection IN (
            'liked-tracks', 'liked-playlists', 'followings', 'followers',
            'owned-tracks', 'owned-playlists',
            'track-favoriters', 'track-reposters', 'playlist-reposters', 'track-comments'
        )
    ),
    ADD CONSTRAINT catalog_collection_public_scope CHECK (
        collection NOT IN (
            'followers', 'track-favoriters', 'track-reposters', 'playlist-reposters',
            'track-comments'
        )
        OR scope = 'public'
    );

CREATE TABLE track_comments (
    id uuid PRIMARY KEY,
    sc_track_id text NOT NULL CHECK (sc_track_id ~ '^[1-9][0-9]*$'),
    sc_comment_id text CHECK (sc_comment_id ~ '^[1-9][0-9]*$'),
    user_urn text NOT NULL,
    body text NOT NULL CHECK (octet_length(body) <= 16384),
    track_position_ms bigint CHECK (track_position_ms IS NULL OR track_position_ms >= 0),
    sc_created_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    synced_at timestamptz,
    UNIQUE (sc_comment_id)
);

CREATE INDEX track_comments_page_idx
    ON track_comments (sc_track_id, created_at DESC, id DESC);

CREATE INDEX track_comments_pending_idx
    ON track_comments (sc_track_id, user_urn) WHERE sc_comment_id IS NULL;
