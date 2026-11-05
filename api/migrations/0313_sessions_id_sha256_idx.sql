-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS sessions_id_sha256_idx ON public.sessions (sha256(uuid_send(id)));
