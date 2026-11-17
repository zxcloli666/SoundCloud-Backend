SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.video_accounts (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    source text NOT NULL,
    name text NOT NULL,
    secret text NOT NULL,
    status text NOT NULL DEFAULT 'unchecked',
    last_checked_at timestamptz,
    last_error text,
    alerted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT video_accounts_name_key UNIQUE (name),
    CONSTRAINT video_accounts_source_valid CHECK (source IN ('youtube', 'spotify', 'yandex')),
    CONSTRAINT video_accounts_status_valid CHECK (status IN ('ok', 'dead', 'unchecked')),
    CONSTRAINT video_accounts_secret_valid CHECK (secret <> '')
);

CREATE TABLE IF NOT EXISTS public.track_videos (
    sc_track_id text NOT NULL,
    kind text NOT NULL,
    source text NOT NULL,
    source_ref text,
    s3_key text NOT NULL,
    offset_ms integer NOT NULL DEFAULT 0,
    loop boolean NOT NULL DEFAULT false,
    duration_ms integer NOT NULL,
    verify jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT track_videos_pkey PRIMARY KEY (sc_track_id, kind),
    CONSTRAINT track_videos_kind_valid CHECK (kind IN ('clip', 'loop', 'generated')),
    CONSTRAINT track_videos_window_valid CHECK (offset_ms >= 0 AND duration_ms > 0)
);

ALTER TABLE public.track_videos
    ADD CONSTRAINT track_videos_track_fk
    FOREIGN KEY (sc_track_id) REFERENCES public.tracks(sc_track_id) ON DELETE CASCADE NOT VALID;

CREATE TABLE IF NOT EXISTS public.track_video_dispatch (
    sc_track_id text PRIMARY KEY,
    stage text NOT NULL,
    attempts integer NOT NULL DEFAULT 0,
    next_at timestamptz NOT NULL DEFAULT now(),
    last_error text,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT track_video_dispatch_attempts_valid CHECK (attempts >= 0),
    CONSTRAINT track_video_dispatch_stage_valid CHECK (stage <> '')
);

ALTER TABLE public.track_video_dispatch
    ADD CONSTRAINT track_video_dispatch_track_fk
    FOREIGN KEY (sc_track_id) REFERENCES public.tracks(sc_track_id) ON DELETE CASCADE NOT VALID;

CREATE INDEX IF NOT EXISTS track_video_dispatch_due_idx
    ON public.track_video_dispatch (stage, next_at);

CREATE TABLE IF NOT EXISTS public.video_flags (
    kind text NOT NULL,
    sc_user_id text NOT NULL,
    added_by text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT video_flags_pkey PRIMARY KEY (kind, sc_user_id),
    CONSTRAINT video_flags_kind_valid CHECK (kind IN ('artist', 'account'))
);
