-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS user_likes_tracks_intent_idx ON public.user_likes_tracks (wanted_v) WHERE NOT wanted_state OR synced_for_v < wanted_v;
