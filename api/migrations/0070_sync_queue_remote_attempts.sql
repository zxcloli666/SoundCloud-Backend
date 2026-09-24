ALTER TABLE sync_queue
    ADD COLUMN remote_attempted_generation bigint;

UPDATE sync_queue
SET remote_attempted_generation = remote_completed_generation
WHERE remote_completed_generation IS NOT NULL;

ALTER TABLE sync_queue
    ADD CONSTRAINT sync_queue_remote_attempt_generation_positive CHECK (
        remote_attempted_generation IS NULL OR remote_attempted_generation > 0
    ),
    ADD CONSTRAINT sync_queue_remote_completion_was_attempted CHECK (
        remote_completed_generation IS NULL
        OR remote_attempted_generation = remote_completed_generation
    );

CREATE INDEX IF NOT EXISTS user_likes_tracks_sync_heal_idx
    ON user_likes_tracks (COALESCE(synced_at, created_at), user_id, sc_track_id)
    WHERE progress = true;

CREATE INDEX IF NOT EXISTS user_likes_playlists_sync_heal_idx
    ON user_likes_playlists (COALESCE(synced_at, created_at), user_id, playlist_urn)
    WHERE progress = true;

CREATE INDEX IF NOT EXISTS user_followings_sync_heal_idx
    ON user_followings (COALESCE(synced_at, created_at), user_id, target_user_urn)
    WHERE progress = true;
