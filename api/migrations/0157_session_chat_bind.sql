SET LOCAL lock_timeout = '2s';

ALTER TABLE login_requests ADD COLUMN IF NOT EXISTS chat_bind bytea;
ALTER TABLE link_requests ADD COLUMN IF NOT EXISTS chat_bind bytea;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS chat_bind bytea;
