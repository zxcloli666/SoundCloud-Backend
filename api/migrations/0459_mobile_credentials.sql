SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.mobile_credentials (
    soundcloud_user_id text PRIMARY KEY,
    oauth_app_id uuid NOT NULL REFERENCES public.oauth_apps (id) ON DELETE RESTRICT,
    device_id uuid NOT NULL,
    access_token text NOT NULL,
    refresh_token text NOT NULL,
    expires_at timestamptz NOT NULL,
    scope text NOT NULL,
    linked_at timestamptz NOT NULL,
    token_v bigint NOT NULL DEFAULT 0,
    status text NOT NULL DEFAULT 'active',
    link_v bigint NOT NULL DEFAULT 0,
    refresh_generation bigint NOT NULL DEFAULT 1,
    refresh_failure_count integer NOT NULL DEFAULT 0,
    refresh_lease_id uuid,
    refresh_lease_expires_at timestamptz,
    last_refresh_attempt_at timestamptz,
    last_refresh_success_at timestamptz,
    last_refresh_error_kind varchar(32),
    last_refresh_error text,
    retry_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    plan text,
    plan_seen_at timestamptz,
    plan_v bigint NOT NULL DEFAULT 0,
    uq_opt_in boolean NOT NULL DEFAULT false,
    consent_v bigint NOT NULL DEFAULT 0,
    uq_busy_until timestamptz,
    uq_cooldown_until timestamptz,
    uq_failures integer NOT NULL DEFAULT 0,
    CONSTRAINT mobile_credentials_status_check CHECK (status IN ('active', 'relink_required')),
    CONSTRAINT mobile_credentials_plan_check CHECK (char_length(plan) <= 64)
);

CREATE INDEX IF NOT EXISTS mobile_credentials_oauth_app_idx ON public.mobile_credentials (oauth_app_id);

CREATE TABLE IF NOT EXISTS public.mobile_link_requests (
    id uuid PRIMARY KEY,
    state text NOT NULL,
    code_verifier text NOT NULL,
    oauth_app_id uuid NOT NULL,
    session_id uuid NOT NULL,
    expected_soundcloud_user_id text NOT NULL,
    device_id uuid NOT NULL,
    status varchar(16) NOT NULL DEFAULT 'pending',
    error varchar(32),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    row_v bigint NOT NULL DEFAULT 0,
    CONSTRAINT mobile_link_requests_state_uq UNIQUE (state),
    CONSTRAINT mobile_link_requests_status_check
        CHECK (status IN ('pending', 'processing', 'completed', 'failed', 'expired', 'account_mismatch'))
);

CREATE INDEX IF NOT EXISTS mobile_link_requests_expires_idx ON public.mobile_link_requests (expires_at);
