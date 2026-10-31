-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS cmd_outbox_login_idx ON public.cmd_outbox (created_at) WHERE kind = 'connection.login';
