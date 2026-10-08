SET LOCAL lock_timeout = '5s';

ALTER TABLE background_jobs VALIDATE CONSTRAINT background_jobs_lane_valid;

ALTER TABLE background_job_failures VALIDATE CONSTRAINT background_job_failures_lane_valid;
