DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'soundcloud_connections'
          AND column_name = 'uses_environment_oauth_app'
    ) THEN
        IF EXISTS (
            SELECT 1
            FROM soundcloud_connections
            WHERE uses_environment_oauth_app
        ) THEN
            RAISE EXCEPTION 'environment OAuth connections must be mapped before migration 0066';
        END IF;
    END IF;
END
$$;

ALTER TABLE soundcloud_connections
    DROP CONSTRAINT IF EXISTS soundcloud_connections_oauth_source_valid,
    DROP COLUMN IF EXISTS uses_environment_oauth_app;
