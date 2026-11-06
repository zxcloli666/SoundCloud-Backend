SET LOCAL lock_timeout = '2s';

ALTER TABLE chat.conversations DROP CONSTRAINT IF EXISTS conversations_kind_check;
ALTER TABLE chat.conversations ADD CONSTRAINT conversations_kind_check CHECK (kind IN ('direct', 'group', 'room', 'self', 'public'));

ALTER TABLE chat.members ADD COLUMN IF NOT EXISTS joined_at timestamptz;

CREATE TABLE IF NOT EXISTS chat.public_mutes (
    user_id text PRIMARY KEY,
    until timestamptz NOT NULL,
    by text NOT NULL,
    reason text,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS public_mutes_until ON chat.public_mutes (until);

CREATE INDEX IF NOT EXISTS admin_audit_public ON chat.admin_audit (id DESC) WHERE action LIKE 'public.%';

INSERT INTO chat.conversations (id, kind, created_by, settings, member_count)
VALUES ('00000000-0000-7000-8000-7075626c6963', 'public', '1275950656', '{"slow_mode_s": 5}', 1)
ON CONFLICT (id) DO NOTHING;

INSERT INTO chat.members (conversation_id, user_id, role, state, accepted, notify, joined_at)
VALUES ('00000000-0000-7000-8000-7075626c6963', '1275950656', 'owner', 'active', true, 'mentions', now())
ON CONFLICT (conversation_id, user_id) DO NOTHING;
