CREATE TABLE IF NOT EXISTS public.cmd_dead (
    node text NOT NULL,
    id uuid NOT NULL,
    kind text,
    args jsonb,
    sqlstate text,
    message text,
    at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (node, id)
);

CREATE TABLE IF NOT EXISTS public.cmd_done (
    node text NOT NULL,
    id uuid NOT NULL,
    status text,
    at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (node, id)
);

CREATE TABLE IF NOT EXISTS public.cmd_outbox (
    node text NOT NULL,
    id uuid NOT NULL,
    kind text,
    args jsonb,
    touched jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    done_at timestamptz,
    PRIMARY KEY (node, id)
);

CREATE TABLE IF NOT EXISTS public.cmd_pending (
    tbl text NOT NULL,
    pk jsonb NOT NULL,
    node text,
    id uuid NOT NULL,
    PRIMARY KEY (tbl, pk, id)
);

CREATE TABLE IF NOT EXISTS public.cmd_purge (
    node text NOT NULL,
    id uuid NOT NULL,
    tbl text NOT NULL,
    pk jsonb NOT NULL,
    PRIMARY KEY (node, id, tbl, pk)
);

CREATE TABLE IF NOT EXISTS public.cmd_reassert (
    node text NOT NULL,
    id uuid NOT NULL,
    tbl text NOT NULL,
    pk jsonb NOT NULL,
    "row" jsonb,
    PRIMARY KEY (node, id, tbl, pk)
);

CREATE TABLE IF NOT EXISTS public.entity_reads (
    entity_urn text NOT NULL,
    last_read_at timestamptz NOT NULL,
    PRIMARY KEY (entity_urn)
);

CREATE TABLE IF NOT EXISTS public.job_status (
    kind text NOT NULL,
    dedup_key text NOT NULL,
    state text,
    available_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (kind, dedup_key)
);

CREATE TABLE IF NOT EXISTS public.repl_checksums (
    node text NOT NULL,
    tbl text NOT NULL,
    range text NOT NULL,
    hash text,
    at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (node, tbl, range)
);

CREATE TABLE IF NOT EXISTS public.repl_purges (
    tbl text NOT NULL,
    pk jsonb NOT NULL,
    purged_v bigint,
    at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tbl, pk, at)
);

CREATE TABLE IF NOT EXISTS public.repl_repair (
    id uuid NOT NULL,
    src_node text,
    tbl text,
    pk jsonb,
    "row" jsonb,
    at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (id)
);

CREATE TABLE IF NOT EXISTS public.repl_resend (
    tbl text NOT NULL,
    pk jsonb NOT NULL,
    PRIMARY KEY (tbl, pk)
);

CREATE TABLE IF NOT EXISTS public.repl_tombstones (
    tbl text NOT NULL,
    pk jsonb NOT NULL,
    v bigint NOT NULL DEFAULT 0,
    at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tbl, pk)
);

ALTER TABLE public.cdn_tracks ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.diag_issue_days ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.discover_promoted ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.discover_settings ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.disliked_tracks ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.featured_items ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.link_requests ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.login_requests ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.oauth_app_request_cooldowns ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.oauth_apps ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.sc_egress_health ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.sessions ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.soundcloud_connections ADD COLUMN IF NOT EXISTS refresh_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.soundcloud_connections ADD COLUMN IF NOT EXISTS token_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.subscriptions ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.sync_queue ADD COLUMN IF NOT EXISTS cancelled boolean NOT NULL DEFAULT false;

ALTER TABLE public.sync_queue ADD COLUMN IF NOT EXISTS exec_generation bigint;

ALTER TABLE public.sync_queue ADD COLUMN IF NOT EXISTS intent_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.sync_queue ADD COLUMN IF NOT EXISTS result_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.track_artist_blocks ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_auras ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_blocked_artists ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_followings ADD COLUMN IF NOT EXISTS result_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_followings ADD COLUMN IF NOT EXISTS synced_for_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_followings ADD COLUMN IF NOT EXISTS wanted_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_like_ranks ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_likes_playlists ADD COLUMN IF NOT EXISTS result_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_likes_playlists ADD COLUMN IF NOT EXISTS synced_for_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_likes_playlists ADD COLUMN IF NOT EXISTS wanted_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_likes_tracks ADD COLUMN IF NOT EXISTS result_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_likes_tracks ADD COLUMN IF NOT EXISTS synced_for_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_likes_tracks ADD COLUMN IF NOT EXISTS wanted_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_superlike_ranks ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;

ALTER TABLE public.user_superlikes ADD COLUMN IF NOT EXISTS row_v bigint NOT NULL DEFAULT 0;
