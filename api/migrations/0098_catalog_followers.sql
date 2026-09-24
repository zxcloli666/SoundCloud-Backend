ALTER TABLE catalog_collection_sync
    DROP CONSTRAINT catalog_collection_sync_collection_check;
ALTER TABLE catalog_collection_sync
    ADD CONSTRAINT catalog_collection_sync_collection_check CHECK (
        collection IN ('liked-tracks', 'liked-playlists', 'followings', 'followers', 'owned-tracks', 'owned-playlists')
    ),
    ADD CONSTRAINT catalog_followers_public_scope CHECK (collection <> 'followers' OR scope = 'public');

CREATE TABLE user_followers (
    user_id text NOT NULL,
    target_user_urn text NOT NULL,
    progress boolean NOT NULL DEFAULT false,
    synced_at timestamptz,
    last_read_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, target_user_urn)
);

CREATE INDEX user_followers_page_idx ON user_followers (user_id, created_at DESC, target_user_urn DESC);
