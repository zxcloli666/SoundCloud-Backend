CREATE INDEX login_requests_expiry_idx
    ON login_requests (expires_at, id);

CREATE INDEX link_requests_expiry_idx
    ON link_requests (expires_at, id);

CREATE INDEX sync_queue_dead_failed_idx
    ON sync_queue (failed_at, id)
    WHERE dead = true;
