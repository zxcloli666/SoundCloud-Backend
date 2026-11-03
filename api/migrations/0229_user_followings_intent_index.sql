-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS user_followings_intent_idx ON public.user_followings (wanted_v) WHERE NOT wanted_state OR synced_for_v < wanted_v;
