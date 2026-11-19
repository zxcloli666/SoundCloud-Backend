SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.track_video_gen (
    sc_track_id text PRIMARY KEY,
    stage text NOT NULL,
    attempt integer NOT NULL DEFAULT 1,
    step integer NOT NULL DEFAULT 0,
    shot integer NOT NULL DEFAULT 0,
    redo integer NOT NULL DEFAULT 0,
    faults integer NOT NULL DEFAULT 0,
    sent_at timestamptz,
    next_at timestamptz NOT NULL DEFAULT now(),
    plan jsonb,
    state jsonb NOT NULL DEFAULT '{}'::jsonb,
    rejections jsonb NOT NULL DEFAULT '[]'::jsonb,
    reason text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT track_video_gen_stage_valid CHECK (stage IN (
        'plan', 'keyframe', 'keyframe_review', 'shot', 'shot_review', 'assemble',
        'done', 'rejected', 'failed'
    )),
    CONSTRAINT track_video_gen_counts_valid CHECK (
        attempt >= 1 AND step >= 0 AND shot >= 0 AND redo >= 0 AND faults >= 0
    )
);

CREATE INDEX IF NOT EXISTS track_video_gen_due_idx
    ON public.track_video_gen (stage, next_at)
    WHERE sent_at IS NULL;

CREATE TABLE IF NOT EXISTS public.video_gen_spend (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    period date NOT NULL,
    sc_track_id text NOT NULL,
    call text NOT NULL,
    provider text NOT NULL,
    model text NOT NULL,
    input_tokens bigint NOT NULL DEFAULT 0,
    output_tokens bigint NOT NULL DEFAULT 0,
    cache_read_tokens bigint NOT NULL DEFAULT 0,
    cache_write_tokens bigint NOT NULL DEFAULT 0,
    usd numeric(12, 6) NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT video_gen_spend_usd_valid CHECK (usd >= 0)
);

CREATE INDEX IF NOT EXISTS video_gen_spend_period_idx
    ON public.video_gen_spend (period);

CREATE TABLE IF NOT EXISTS public.video_gen_pause (
    id smallint PRIMARY KEY DEFAULT 1,
    reason text NOT NULL,
    paused_until timestamptz NOT NULL,
    alerted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT video_gen_pause_single CHECK (id = 1),
    CONSTRAINT video_gen_pause_reason_valid CHECK (reason IN ('budget', 'api_down'))
);
