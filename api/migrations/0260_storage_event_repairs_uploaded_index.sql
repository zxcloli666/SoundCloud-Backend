-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS storage_event_repairs_uploaded_at_idx ON public.storage_event_repairs (uploaded_at);
