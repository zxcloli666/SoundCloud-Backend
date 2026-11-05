SET LOCAL lock_timeout = '2s';

CREATE TABLE IF NOT EXISTS chat.media (
    id uuid PRIMARY KEY,
    store smallint NOT NULL,
    object_key text NOT NULL,
    owner_user text NOT NULL,
    origin_conversation uuid NOT NULL,
    kind text NOT NULL CHECK (kind IN ('image', 'thumb', 'avatar')),
    parent_id uuid,
    upload_id uuid,
    bytes bigint NOT NULL CHECK (bytes > 0),
    status text NOT NULL CHECK (status IN ('reserved', 'uploaded', 'attached', 'held', 'deleting', 'lost')),
    lease_until timestamptz,
    attempts smallint NOT NULL DEFAULT 0,
    restorable_until timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    delete_after timestamptz,
    UNIQUE (store, object_key)
);

CREATE UNIQUE INDEX IF NOT EXISTS media_reserved_upload ON chat.media (owner_user, upload_id) WHERE status = 'reserved' AND kind <> 'thumb';
CREATE INDEX IF NOT EXISTS media_deleting ON chat.media (lease_until NULLS FIRST) WHERE status = 'deleting';
CREATE INDEX IF NOT EXISTS media_expiring ON chat.media (delete_after) WHERE status IN ('reserved', 'uploaded');
CREATE INDEX IF NOT EXISTS media_held ON chat.media (id) WHERE status = 'held';
CREATE INDEX IF NOT EXISTS media_owner ON chat.media (owner_user, created_at) WHERE kind <> 'thumb';
CREATE INDEX IF NOT EXISTS media_origin ON chat.media (origin_conversation) WHERE kind <> 'thumb';
CREATE INDEX IF NOT EXISTS media_parent ON chat.media (parent_id) WHERE parent_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS media_fill ON chat.media (store) INCLUDE (bytes) WHERE status <> 'lost';

CREATE TABLE IF NOT EXISTS chat.media_refs (
    media_id uuid NOT NULL,
    conversation_id uuid NOT NULL,
    seq bigint NOT NULL,
    PRIMARY KEY (media_id, conversation_id, seq)
);

CREATE INDEX IF NOT EXISTS media_refs_conversation ON chat.media_refs (conversation_id, seq);

CREATE TABLE IF NOT EXISTS chat.media_holds (
    media_id uuid NOT NULL,
    report_id uuid NOT NULL,
    until timestamptz NOT NULL,
    PRIMARY KEY (media_id, report_id)
);

CREATE INDEX IF NOT EXISTS media_holds_report ON chat.media_holds (report_id);
CREATE INDEX IF NOT EXISTS media_holds_until ON chat.media_holds (until);

CREATE TABLE IF NOT EXISTS chat.media_trim (
    conversation_id uuid PRIMARY KEY,
    queued_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS chat.usage (
    user_id text PRIMARY KEY,
    bytes bigint NOT NULL DEFAULT 0 CHECK (bytes >= 0),
    reserved bigint NOT NULL DEFAULT 0 CHECK (reserved >= 0),
    files integer NOT NULL DEFAULT 0 CHECK (files >= 0),
    day_bytes bigint NOT NULL DEFAULT 0,
    day date NOT NULL DEFAULT current_date,
    updated_at timestamptz NOT NULL DEFAULT now()
);
