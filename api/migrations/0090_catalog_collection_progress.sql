CREATE TABLE IF NOT EXISTS catalog_collection_sync (
    subject_id text NOT NULL,
    collection text NOT NULL CHECK (collection IN ('liked-tracks', 'liked-playlists', 'followings', 'owned-tracks', 'owned-playlists')),
    scope text NOT NULL CHECK (scope IN ('owner', 'public')),
    job_id uuid NOT NULL,
    generation bigint NOT NULL,
    snapshot_id uuid NOT NULL,
    started_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    synced_at timestamptz,
    next_cursor text,
    page_count bigint NOT NULL DEFAULT 0 CHECK (page_count >= 0),
    item_count bigint NOT NULL DEFAULT 0 CHECK (item_count >= 0),
    complete boolean NOT NULL DEFAULT false,
    PRIMARY KEY (subject_id, collection, scope)
);

CREATE TABLE IF NOT EXISTS catalog_collection_seen (
    subject_id text NOT NULL,
    collection text NOT NULL,
    scope text NOT NULL,
    entity_key text NOT NULL,
    snapshot_id uuid NOT NULL,
    PRIMARY KEY (subject_id, collection, scope, entity_key),
    FOREIGN KEY (subject_id, collection, scope)
        REFERENCES catalog_collection_sync (subject_id, collection, scope) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS catalog_collection_cursors (
    subject_id text NOT NULL,
    collection text NOT NULL,
    scope text NOT NULL,
    cursor text NOT NULL,
    PRIMARY KEY (subject_id, collection, scope, cursor),
    FOREIGN KEY (subject_id, collection, scope)
        REFERENCES catalog_collection_sync (subject_id, collection, scope) ON DELETE CASCADE
);
