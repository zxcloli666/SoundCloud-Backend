SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_job_enqueue(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    claim jsonb := args -> 'claim';
    stamp timestamptz;
    inserted bigint;
BEGIN
    IF jsonb_typeof(claim) = 'object' THEN
        stamp := (claim ->> 'at')::timestamptz;
        UPDATE public.playlist_membership_state AS state
        SET next_reconcile_at = stamp + interval '30 seconds',
            updated_at = stamp
        WHERE state.playlist_urn = claim ->> 'playlist_urn'
          AND EXISTS (SELECT 1 FROM public.playlists p WHERE p.urn = state.playlist_urn AND p.deleted_at IS NULL)
          AND state.next_reconcile_at <= stamp;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('touched', '[]'::jsonb,
                'output', jsonb_build_object('claimed', false, 'inserted', 0));
        END IF;
    END IF;

    WITH wanted AS (
        SELECT DISTINCT ON (j.kind, j.dedup_key)
               j.id, j.kind, j.lane, j.dedup_key, j.payload, j.max_attempts, j.raise_priority,
               j.priority + CASE
                   WHEN EXISTS (
                       SELECT 1 FROM public.subscriptions AS subscription
                       WHERE subscription.user_urn = ANY (j.star_candidates)
                         AND subscription.exp_date > extract(epoch FROM now())::bigint
                   ) THEN j.star_bonus
                   ELSE 0::smallint
               END AS priority
        FROM jsonb_to_recordset(args -> 'jobs') AS j(
            id uuid, kind text, lane text, dedup_key text, payload jsonb, priority smallint,
            max_attempts smallint, raise_priority boolean, star_candidates text[], star_bonus smallint
        )
        ORDER BY j.kind, j.dedup_key, j.priority DESC
    ), raised AS (
        INSERT INTO public.background_jobs (id, kind, lane, dedup_key, payload, priority, max_attempts)
        SELECT w.id, w.kind, w.lane, w.dedup_key, w.payload, w.priority, w.max_attempts
        FROM wanted AS w
        WHERE w.raise_priority
        ON CONFLICT (kind, dedup_key) WHERE dedup_key IS NOT NULL DO UPDATE
        SET priority = EXCLUDED.priority
        WHERE background_jobs.priority < EXCLUDED.priority
        RETURNING kind, dedup_key, available_at, (xmax = 0) AS fresh
    ), kept AS (
        INSERT INTO public.background_jobs (id, kind, lane, dedup_key, payload, priority, max_attempts)
        SELECT w.id, w.kind, w.lane, w.dedup_key, w.payload, w.priority, w.max_attempts
        FROM wanted AS w
        WHERE NOT w.raise_priority
        ON CONFLICT (kind, dedup_key) WHERE dedup_key IS NOT NULL DO NOTHING
        RETURNING kind, dedup_key, available_at, true AS fresh
    ), created AS (
        SELECT * FROM raised WHERE fresh
        UNION ALL
        SELECT * FROM kept
    ), status AS (
        INSERT INTO public.job_status (kind, dedup_key, state, available_at, updated_at)
        SELECT c.kind, c.dedup_key, 'queued', c.available_at, now()
        FROM created AS c
        WHERE c.kind = ANY (public.scd_job_status_kinds())
        ON CONFLICT (kind, dedup_key) DO NOTHING
        RETURNING 1
    )
    SELECT count(*) INTO inserted FROM created;

    RETURN jsonb_build_object('touched', '[]'::jsonb,
        'output', jsonb_build_object('claimed', true, 'inserted', inserted));
END
$f$;
