CREATE TABLE admin_maintenance_runs (
    kind varchar(64) PRIMARY KEY,
    run_id uuid NOT NULL,
    status varchar(16) NOT NULL,
    phase varchar(32) NOT NULL,
    cursor_uuid uuid,
    cursor_text text,
    scanned bigint NOT NULL DEFAULT 0,
    changed bigint NOT NULL DEFAULT 0,
    merged bigint NOT NULL DEFAULT 0,
    skipped bigint NOT NULL DEFAULT 0,
    started_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    CONSTRAINT admin_maintenance_runs_kind_valid CHECK (
        kind IN ('catalog_renormalize', 'musicbrainz_names')
    ),
    CONSTRAINT admin_maintenance_runs_status_valid CHECK (
        status IN ('running', 'completed')
    ),
    CONSTRAINT admin_maintenance_runs_counts_valid CHECK (
        scanned >= 0 AND changed >= 0 AND merged >= 0 AND skipped >= 0
    ),
    CONSTRAINT admin_maintenance_runs_completed_valid CHECK (
        (status = 'running' AND completed_at IS NULL)
        OR (status = 'completed' AND completed_at IS NOT NULL)
    )
);
