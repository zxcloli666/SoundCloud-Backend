-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS cmd_outbox_mobile_link_idx ON public.cmd_outbox (created_at) WHERE kind = 'mobile_credential.link';
