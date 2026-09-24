-- no-transaction
CREATE UNIQUE INDEX CONCURRENTLY rec_impressions_event_id_idx ON rec_impressions (event_id) WHERE event_id IS NOT NULL;
