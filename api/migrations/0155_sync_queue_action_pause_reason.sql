SET LOCAL lock_timeout = '5s';

ALTER TABLE sync_queue_action_pauses
    ADD COLUMN IF NOT EXISTS reason text NOT NULL DEFAULT 'rate_limited';

ALTER TABLE sync_queue_action_pauses
    ADD CONSTRAINT sync_queue_action_pauses_reason_valid
    CHECK (reason IN ('rate_limited', 'subscription_required'));
