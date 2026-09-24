ALTER TABLE user_likes_tracks
    ADD COLUMN IF NOT EXISTS liked_at timestamptz;
