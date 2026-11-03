SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.oauth_app_tokens_local (
    oauth_app_id uuid PRIMARY KEY,
    generation uuid NOT NULL DEFAULT gen_random_uuid(),
    access_token text NOT NULL,
    expires_at timestamptz NOT NULL,
    scope text,
    issued_at timestamptz NOT NULL DEFAULT now()
);
