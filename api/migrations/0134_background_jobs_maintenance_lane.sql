ALTER TABLE background_jobs DROP CONSTRAINT IF EXISTS background_jobs_lane_valid;
ALTER TABLE background_jobs
    ADD CONSTRAINT background_jobs_lane_valid
    CHECK (lane IN ('core_fast', 'core_bulk', 'maintenance', 'ops')) NOT VALID;

ALTER TABLE background_job_failures DROP CONSTRAINT IF EXISTS background_job_failures_lane_valid;
ALTER TABLE background_job_failures
    ADD CONSTRAINT background_job_failures_lane_valid
    CHECK (lane IN ('core_fast', 'core_bulk', 'maintenance', 'ops')) NOT VALID;

ALTER TABLE background_schedules DROP CONSTRAINT IF EXISTS background_schedules_lane_valid;
ALTER TABLE background_schedules
    ADD CONSTRAINT background_schedules_lane_valid
    CHECK (lane IN ('core_fast', 'core_bulk', 'maintenance', 'ops'));

UPDATE background_schedules
SET lane = 'maintenance',
    updated_at = now()
WHERE lane <> 'maintenance'
  AND kind IN (
      'enrich.revalidate_attribution',
      'catalog.review_credits',
      'catalog.reconcile_works',
      'collab.bootstrap',
      'collab.train',
      'discover.accounts',
      'discover.aggregates',
      'discover.catalog_genius',
      'discover.catalog_musicbrainz',
      'discover.interest',
      'enrich.tracks',
      'indexing.reap',
      'lyrics.lookup_sweep',
      'lyrics.reap_embeddings',
      'lyrics.reap_transcriptions',
      'playlists.legacy_drain',
      'playlists.reconcile_sweep',
      'recommendations.colike',
      'recommendations.quality_backfill',
      'recommendations.quality_train',
      'recommendations.wave_priority',
      'indexing.resolve_durations',
      'enrich.resolve_wanted',
      'subscriptions.snapshot'
  );

UPDATE background_jobs
SET lane = 'maintenance',
    updated_at = now()
WHERE lane <> 'maintenance'
  AND kind IN (
      'enrich.revalidate_attribution',
      'catalog.review_credits',
      'catalog.reconcile_works',
      'collab.bootstrap',
      'collab.train',
      'discover.accounts',
      'discover.aggregates',
      'discover.catalog_genius',
      'discover.catalog_musicbrainz',
      'discover.interest',
      'enrich.tracks',
      'indexing.reap',
      'lyrics.lookup_sweep',
      'lyrics.reap_embeddings',
      'lyrics.reap_transcriptions',
      'playlists.legacy_drain',
      'playlists.reconcile_sweep',
      'recommendations.colike',
      'recommendations.quality_backfill',
      'recommendations.quality_train',
      'recommendations.wave_priority',
      'indexing.resolve_durations',
      'enrich.resolve_wanted',
      'subscriptions.snapshot'
  );

UPDATE background_jobs
SET lane = 'core_fast',
    updated_at = now()
WHERE lane <> 'core_fast'
  AND kind = 'lyrics.embed';
