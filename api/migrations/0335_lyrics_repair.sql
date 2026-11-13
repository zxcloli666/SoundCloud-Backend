SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.lyrics_repair (
  sc_track_id text PRIMARY KEY,
  kind varchar(16) NOT NULL,
  cache_created_at timestamp NOT NULL,
  priority smallint NOT NULL DEFAULT 0,
  status varchar(16) NOT NULL DEFAULT 'pending',
  outcome varchar(32),
  attempts integer NOT NULL DEFAULT 0,
  next_run_at timestamptz NOT NULL DEFAULT now(),
  claim_job_id uuid,
  claim_job_generation bigint,
  claim_job_lease_id uuid,
  claim_expires_at timestamptz,
  old_plain_text text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  settled_at timestamptz,
  CONSTRAINT lyrics_repair_kind_valid CHECK (kind IN ('refetch', 'description')),
  CONSTRAINT lyrics_repair_status_valid CHECK (status IN ('pending', 'done') AND attempts >= 0),
  CONSTRAINT lyrics_repair_claim_valid CHECK (
    (claim_job_id IS NULL AND claim_job_generation IS NULL AND claim_job_lease_id IS NULL AND claim_expires_at IS NULL)
    OR (claim_job_id IS NOT NULL AND claim_job_generation > 0 AND claim_job_lease_id IS NOT NULL AND claim_expires_at IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS lyrics_repair_due_idx ON public.lyrics_repair (priority DESC, next_run_at, sc_track_id)
  WHERE kind = 'refetch' AND status = 'pending' AND claim_job_id IS NULL;

CREATE INDEX IF NOT EXISTS lyrics_repair_expired_claim_idx ON public.lyrics_repair (claim_expires_at, sc_track_id)
  WHERE claim_job_id IS NOT NULL;

ALTER TABLE public.admin_maintenance_runs
    DROP CONSTRAINT admin_maintenance_runs_kind_valid,
    ADD CONSTRAINT admin_maintenance_runs_kind_valid CHECK (
        kind IN ('catalog_renormalize', 'musicbrainz_names', 'wanted_aliases', 'lyrics_lookup_queue', 'track_search', 'lyrics_refetch', 'lyrics_description')
    );
