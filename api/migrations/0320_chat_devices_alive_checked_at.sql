SET LOCAL lock_timeout = '3s';

ALTER TABLE chat.devices ADD COLUMN IF NOT EXISTS alive_checked_at timestamptz;

DO $do$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_constraint AS constraint_row
        WHERE constraint_row.conrelid = 'chat.devices'::regclass
          AND constraint_row.conname = 'devices_session_hash_size'
    ) THEN
        ALTER TABLE chat.devices ADD CONSTRAINT devices_session_hash_size CHECK (octet_length(session_hash) = 32);
    END IF;
END
$do$;

CREATE INDEX IF NOT EXISTS devices_alive_check_idx
    ON chat.devices (alive_checked_at NULLS FIRST, id)
    WHERE status = 'active';
