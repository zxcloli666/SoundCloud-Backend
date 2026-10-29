SET LOCAL lock_timeout = '5s';

ALTER TABLE admin_maintenance_runs
    DROP CONSTRAINT admin_maintenance_runs_kind_valid,
    ADD CONSTRAINT admin_maintenance_runs_kind_valid CHECK (
        kind IN ('catalog_renormalize', 'musicbrainz_names', 'wanted_aliases')
    );
