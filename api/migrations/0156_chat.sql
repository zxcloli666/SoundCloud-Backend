SET LOCAL lock_timeout = '5s';

CREATE SCHEMA IF NOT EXISTS chat;

CREATE TABLE IF NOT EXISTS chat.meta (
    id smallint PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    incarnation uuid NOT NULL
);

INSERT INTO chat.meta (id, incarnation) VALUES (1, gen_random_uuid()) ON CONFLICT (id) DO NOTHING;

CREATE TABLE IF NOT EXISTS chat.users (
    user_id text PRIMARY KEY,
    dm_policy text NOT NULL DEFAULT 'following' CHECK (dm_policy IN ('everyone', 'following', 'nobody')),
    read_receipts boolean NOT NULL DEFAULT true,
    typing boolean NOT NULL DEFAULT true,
    presence_share boolean NOT NULL DEFAULT false,
    restricted_until timestamptz,
    restriction_scope text[] NOT NULL DEFAULT '{}',
    first_seen_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS chat.devices (
    id uuid PRIMARY KEY,
    user_id text NOT NULL,
    session_id uuid NOT NULL,
    status text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'revoked')),
    auth_pub bytea NOT NULL,
    box_pub bytea NOT NULL,
    platform text NOT NULL,
    name text NOT NULL,
    app_version text NOT NULL,
    caps text[] NOT NULL DEFAULT '{}',
    tier text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    history_unlocked_at timestamptz,
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz
);

CREATE UNIQUE INDEX IF NOT EXISTS devices_live_session ON chat.devices (session_id) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS devices_live_user ON chat.devices (user_id) WHERE status = 'active';

CREATE TABLE IF NOT EXISTS chat.conversations (
    id uuid PRIMARY KEY,
    kind text NOT NULL CHECK (kind IN ('direct', 'group', 'room', 'self')),
    dm_key text UNIQUE,
    room_ref text,
    created_by text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_seq bigint NOT NULL DEFAULT 0,
    last_at timestamptz NOT NULL DEFAULT now(),
    key_epoch integer NOT NULL DEFAULT 0,
    epoch_messages integer NOT NULL DEFAULT 0,
    epoch_started_at timestamptz NOT NULL DEFAULT now(),
    rekey boolean NOT NULL DEFAULT false,
    meta_ct bytea,
    meta_epoch integer,
    settings jsonb NOT NULL DEFAULT '{}',
    member_count integer NOT NULL DEFAULT 0,
    ttl_s integer CHECK (ttl_s IN (3600, 86400, 604800)),
    room_alive_until timestamptz,
    deleted_at timestamptz
) WITH (fillfactor = 80);

CREATE INDEX IF NOT EXISTS conversations_room_alive ON chat.conversations (room_alive_until) WHERE kind = 'room' AND deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS conversations_deleted ON chat.conversations (deleted_at) WHERE deleted_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS chat.conversation_keys (
    conversation_id uuid NOT NULL,
    epoch integer NOT NULL,
    wrapped bytea NOT NULL,
    kek_version smallint NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (conversation_id, epoch)
);

CREATE TABLE IF NOT EXISTS chat.members (
    conversation_id uuid NOT NULL,
    user_id text NOT NULL,
    role text NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'member')),
    state text NOT NULL CHECK (state IN ('active', 'invited', 'left', 'kicked', 'banned')),
    accepted boolean NOT NULL DEFAULT false,
    joined_seq bigint NOT NULL DEFAULT 0,
    joined_epoch integer NOT NULL DEFAULT 0,
    last_read_seq bigint NOT NULL DEFAULT 0,
    muted_until timestamptz,
    notify text NOT NULL DEFAULT 'all' CHECK (notify IN ('all', 'mentions', 'none')),
    pinned_at timestamptz,
    archived boolean NOT NULL DEFAULT false,
    hidden_before_seq bigint NOT NULL DEFAULT 0,
    invited_by text,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (conversation_id, user_id)
);

CREATE INDEX IF NOT EXISTS members_live_user ON chat.members (user_id) INCLUDE (conversation_id) WHERE state IN ('active', 'invited');

CREATE TABLE IF NOT EXISTS chat.messages (
    conversation_id uuid NOT NULL,
    seq bigint NOT NULL,
    client_id uuid NOT NULL,
    sender_user text NOT NULL,
    sender_device uuid,
    class text NOT NULL CHECK (class IN ('post', 'edit', 'react', 'pin', 'meta', 'system')),
    epoch integer NOT NULL,
    target_seq bigint,
    body bytea,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz,
    PRIMARY KEY (conversation_id, seq)
);

CREATE UNIQUE INDEX IF NOT EXISTS messages_client ON chat.messages (conversation_id, client_id);
CREATE INDEX IF NOT EXISTS messages_expiring ON chat.messages (expires_at) WHERE expires_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS chat.blocks (
    user_id text NOT NULL,
    blocked_user text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, blocked_user)
);

CREATE INDEX IF NOT EXISTS blocks_blocked ON chat.blocks (blocked_user);

CREATE TABLE IF NOT EXISTS chat.invites (
    token_hash bytea PRIMARY KEY,
    conversation_id uuid NOT NULL,
    created_by text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    uses_left integer,
    revoked_at timestamptz
);

CREATE INDEX IF NOT EXISTS invites_conversation ON chat.invites (conversation_id);

CREATE TABLE IF NOT EXISTS chat.reports (
    id uuid PRIMARY KEY,
    reporter text NOT NULL,
    target_user text NOT NULL,
    conversation_id uuid NOT NULL,
    seq_from bigint NOT NULL,
    seq_to bigint NOT NULL,
    reason text NOT NULL,
    comment text,
    weight real NOT NULL,
    evidence bytea,
    evidence_kek_version smallint,
    state text NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'actioned', 'rejected')),
    created_at timestamptz NOT NULL DEFAULT now(),
    handled_by text,
    handled_at timestamptz,
    purge_after timestamptz
);

CREATE INDEX IF NOT EXISTS reports_open ON chat.reports (created_at) WHERE state = 'open';
CREATE INDEX IF NOT EXISTS reports_target ON chat.reports (target_user, created_at);
CREATE INDEX IF NOT EXISTS reports_purge ON chat.reports (purge_after) WHERE purge_after IS NOT NULL;

CREATE TABLE IF NOT EXISTS chat.admin_audit (
    id uuid PRIMARY KEY,
    admin text NOT NULL,
    action text NOT NULL,
    target text NOT NULL,
    detail jsonb NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now()
);
