-- no-transaction
CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS rec_hard_negatives_event_id_idx ON rec_hard_negatives (event_id) WHERE event_id IS NOT NULL;
