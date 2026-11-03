-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS cdn_tracks_ok_updated_at_idx ON cdn_tracks (updated_at) INCLUDE (track_urn) WHERE status = 'ok' AND quality = 'single';
