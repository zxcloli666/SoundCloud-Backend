ALTER TABLE oauth_app_tokens
    ADD COLUMN refresh_token text,
    ADD CONSTRAINT oauth_app_tokens_refresh_token_present CHECK (
        refresh_token IS NULL OR octet_length(refresh_token) > 0
    );
