ALTER TABLE lyrics_cache
    ADD COLUMN IF NOT EXISTS synced_version varchar(128);

ALTER TABLE lyrics_cache
    ADD CONSTRAINT lyrics_cache_synced_version_valid CHECK (
        synced_version IS NULL
        OR (
            synced_source = 'self_gen'
            AND octet_length(synced_version) BETWEEN 1 AND 128
        )
    ) NOT VALID;

ALTER TABLE lyrics_cache
    VALIDATE CONSTRAINT lyrics_cache_synced_version_valid;
