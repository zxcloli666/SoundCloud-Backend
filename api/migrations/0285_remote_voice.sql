SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.remote_link_codes (
    id uuid PRIMARY KEY,
    sc_user_id text NOT NULL,
    provider text NOT NULL,
    code_hash text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    row_v bigint NOT NULL DEFAULT 0,
    CONSTRAINT remote_link_codes_provider_ck CHECK (provider IN ('alice', 'salut'))
);

CREATE INDEX IF NOT EXISTS remote_link_codes_hash_idx ON public.remote_link_codes (code_hash);
CREATE INDEX IF NOT EXISTS remote_link_codes_user_idx ON public.remote_link_codes (sc_user_id, provider);
CREATE INDEX IF NOT EXISTS remote_link_codes_expires_idx ON public.remote_link_codes (expires_at);

CREATE TABLE IF NOT EXISTS public.remote_grants (
    id uuid PRIMARY KEY,
    sc_user_id text NOT NULL,
    provider text NOT NULL,
    client_id text NOT NULL DEFAULT '',
    external_id text,
    code_hash text,
    code_redirect_uri text,
    code_expires_at timestamptz,
    access_hash text,
    access_expires_at timestamptz,
    refresh_hash text,
    refresh_expires_at timestamptz,
    prev_refresh_hash text,
    rotated_at timestamptz,
    rotation_seed text,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz,
    row_v bigint NOT NULL DEFAULT 0,
    CONSTRAINT remote_grants_provider_ck CHECK (provider IN ('alice', 'salut'))
);

CREATE UNIQUE INDEX IF NOT EXISTS remote_grants_code_uq ON public.remote_grants (code_hash) WHERE code_hash IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS remote_grants_access_uq ON public.remote_grants (access_hash) WHERE access_hash IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS remote_grants_refresh_uq ON public.remote_grants (refresh_hash) WHERE refresh_hash IS NOT NULL;
CREATE INDEX IF NOT EXISTS remote_grants_user_idx ON public.remote_grants (sc_user_id);
CREATE INDEX IF NOT EXISTS remote_grants_external_idx ON public.remote_grants (provider, external_id) WHERE external_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS remote_grants_prev_refresh_idx ON public.remote_grants (prev_refresh_hash) WHERE prev_refresh_hash IS NOT NULL;
