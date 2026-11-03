SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.cmd_dead (
    node text NOT NULL,
    id uuid NOT NULL,
    kind text NOT NULL,
    args jsonb NOT NULL,
    sqlstate text,
    message text,
    at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (node, id)
);

CREATE TABLE IF NOT EXISTS public.cmd_done (
    node text NOT NULL,
    id uuid NOT NULL,
    status text NOT NULL,
    at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (node, id)
);

CREATE TABLE IF NOT EXISTS public.cmd_outbox (
    node text NOT NULL,
    id uuid NOT NULL,
    kind text NOT NULL,
    args jsonb NOT NULL,
    touched jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    done_at timestamptz,
    PRIMARY KEY (node, id)
);

CREATE TABLE IF NOT EXISTS public.cmd_pending (
    tbl text NOT NULL,
    pk jsonb NOT NULL,
    node text NOT NULL,
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
    "row" jsonb NOT NULL,
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
    state text NOT NULL,
    available_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (kind, dedup_key)
);

CREATE TABLE IF NOT EXISTS public.repl_checksums (
    node text NOT NULL,
    tbl text NOT NULL,
    range text NOT NULL,
    hash text NOT NULL,
    at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (node, tbl, range)
);

CREATE TABLE IF NOT EXISTS public.repl_purges (
    tbl text NOT NULL,
    pk jsonb NOT NULL,
    purged_v bigint NOT NULL,
    at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tbl, pk, at)
);

CREATE TABLE IF NOT EXISTS public.repl_repair (
    id uuid NOT NULL,
    src_node text NOT NULL,
    tbl text NOT NULL,
    pk jsonb NOT NULL,
    "row" jsonb NOT NULL,
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

CREATE TABLE IF NOT EXISTS public.scd_unpublished_columns (
    tbl text NOT NULL,
    col text NOT NULL,
    at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tbl, col)
);

CREATE INDEX IF NOT EXISTS cmd_pending_cmd_idx ON public.cmd_pending (node, id);
