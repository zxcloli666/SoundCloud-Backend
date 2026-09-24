ALTER TABLE catalog_collection_sync
    DROP CONSTRAINT catalog_collection_sync_collection_check,
    DROP CONSTRAINT catalog_followers_public_scope;
ALTER TABLE catalog_collection_sync
    ADD CONSTRAINT catalog_collection_sync_collection_check CHECK (
        collection IN (
            'liked-tracks', 'liked-playlists', 'followings', 'followers',
            'owned-tracks', 'owned-playlists',
            'track-favoriters', 'track-reposters', 'playlist-reposters'
        )
    ),
    ADD CONSTRAINT catalog_collection_public_scope CHECK (
        collection NOT IN ('followers', 'track-favoriters', 'track-reposters', 'playlist-reposters')
        OR scope = 'public'
    );

CREATE TABLE catalog_audience (
    subject_urn text NOT NULL,
    relation text NOT NULL CHECK (
        relation IN ('track-favoriters', 'track-reposters', 'playlist-reposters')
    ),
    user_urn text NOT NULL,
    synced_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (subject_urn, relation, user_urn)
);

CREATE INDEX catalog_audience_page_idx
    ON catalog_audience (subject_urn, relation, created_at DESC, user_urn DESC);
