SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.track_seeks (
    id uuid PRIMARY KEY,
    sc_user_id text NOT NULL,
    sc_track_id text NOT NULL,
    position_ms integer NOT NULL,
    from_ms integer NOT NULL,
    source text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT track_seeks_track_ck CHECK (sc_track_id ~ '^[1-9][0-9]*$'),
    CONSTRAINT track_seeks_position_ck CHECK (position_ms >= 0 AND from_ms >= 0),
    CONSTRAINT track_seeks_source_ck CHECK (source IN ('player', 'moment'))
);

CREATE INDEX IF NOT EXISTS track_seeks_track_idx ON public.track_seeks (sc_track_id, created_at);
