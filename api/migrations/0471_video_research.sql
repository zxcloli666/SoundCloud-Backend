SET LOCAL lock_timeout = '5s';

DO $$
BEGIN
    IF public.scd_role() = 'core' THEN
        UPDATE public.track_video_dispatch AS dispatch
        SET next_at = dispatch.updated_at + CASE
                WHEN COALESCE(track.release_date::timestamptz, track.sc_created_at) > now() - interval '180 days'
                    THEN interval '30 days'
                ELSE interval '90 days'
            END
        FROM public.tracks AS track
        WHERE track.sc_track_id = dispatch.sc_track_id
          AND dispatch.stage = 'found'
          AND dispatch.next_at <= dispatch.updated_at;
    END IF;
END
$$;
