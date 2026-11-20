SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.user_lyrics (
    soundcloud_user_id text NOT NULL,
    sc_track_id text NOT NULL,
    plain_text text,
    synced_lrc text,
    origin varchar(16) NOT NULL,
    lrclib_id bigint,
    updated_at timestamptz NOT NULL DEFAULT now(),
    row_v bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (soundcloud_user_id, sc_track_id),
    CONSTRAINT user_lyrics_text_present CHECK (
        NULLIF(btrim(plain_text), '') IS NOT NULL
        OR NULLIF(btrim(synced_lrc), '') IS NOT NULL
    ),
    CONSTRAINT user_lyrics_size_ck CHECK (
        coalesce(octet_length(plain_text), 0) <= 65536
        AND coalesce(octet_length(synced_lrc), 0) <= 65536
    ),
    CONSTRAINT user_lyrics_origin_ck CHECK (
        (origin = 'pasted' AND lrclib_id IS NULL)
        OR (origin = 'lrclib' AND coalesce(lrclib_id, 0) > 0)
    )
);
