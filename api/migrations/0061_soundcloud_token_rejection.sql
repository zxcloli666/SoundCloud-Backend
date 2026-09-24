ALTER TABLE soundcloud_connections
    DROP CONSTRAINT soundcloud_connections_refresh_error_valid;

ALTER TABLE soundcloud_connections
    ADD CONSTRAINT soundcloud_connections_refresh_error_valid CHECK (
        last_refresh_error_kind IS NULL
        OR last_refresh_error_kind IN (
            'rate_limited',
            'reauthorization_required',
            'temporarily_unavailable',
            'timed_out',
            'token_rejected'
        )
    );
