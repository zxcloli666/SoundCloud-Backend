SET LOCAL lock_timeout = '5s';

ALTER TABLE background_jobs DROP CONSTRAINT IF EXISTS background_jobs_lane_valid;
ALTER TABLE background_jobs
    ADD CONSTRAINT background_jobs_lane_valid
    CHECK (lane IN ('core_fast', 'core_bulk', 'lookup', 'maintenance', 'ops')) NOT VALID;

ALTER TABLE background_job_failures DROP CONSTRAINT IF EXISTS background_job_failures_lane_valid;
ALTER TABLE background_job_failures
    ADD CONSTRAINT background_job_failures_lane_valid
    CHECK (lane IN ('core_fast', 'core_bulk', 'lookup', 'maintenance', 'ops')) NOT VALID;

ALTER TABLE background_schedules DROP CONSTRAINT IF EXISTS background_schedules_lane_valid;
ALTER TABLE background_schedules
    ADD CONSTRAINT background_schedules_lane_valid
    CHECK (lane IN ('core_fast', 'core_bulk', 'lookup', 'maintenance', 'ops'));
