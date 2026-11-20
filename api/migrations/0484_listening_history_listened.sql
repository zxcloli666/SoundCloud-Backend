SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.listening_history_listened (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    history_id uuid NOT NULL,
    soundcloud_user_id text NOT NULL,
    listened_ms integer NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT listening_history_listened_ms_ck CHECK (listened_ms >= 0)
);

CREATE INDEX IF NOT EXISTS listening_history_listened_history_idx
    ON public.listening_history_listened (history_id) INCLUDE (listened_ms);

CREATE INDEX IF NOT EXISTS listening_history_listened_user_idx
    ON public.listening_history_listened (soundcloud_user_id);
