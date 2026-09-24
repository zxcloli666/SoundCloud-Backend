DO $$
DECLARE
    conflicting_identity text;
BEGIN
    SELECT account.sc_user_id INTO conflicting_identity
    FROM artist_sc_accounts AS account
    WHERE account.verified
    GROUP BY account.sc_user_id
    HAVING count(*) > 1
    ORDER BY account.sc_user_id
    LIMIT 1;

    IF conflicting_identity IS NOT NULL THEN
        RAISE EXCEPTION USING
            ERRCODE = 'unique_violation',
            MESSAGE = format(
                'verified SoundCloud account is claimed by more than one artist and must be resolved before migration: %s',
                conflicting_identity
            );
    END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS artist_sc_accounts_verified_identity_uq
    ON artist_sc_accounts (sc_user_id)
    WHERE verified;

CREATE INDEX IF NOT EXISTS artist_sc_accounts_identity_idx
    ON artist_sc_accounts (sc_user_id, artist_id)
    WHERE verified OR source = 'mb_resolve';

CREATE OR REPLACE FUNCTION artist_sc_account_flag() RETURNS trigger AS
$$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF (NEW.verified OR NEW.source = 'mb_resolve') AND NEW.role IN ('main', 'alt', 'demo') THEN
            UPDATE artists
            SET has_sc_account = true
            WHERE id = NEW.artist_id
              AND NOT has_sc_account;
        END IF;
        RETURN NULL;
    END IF;

    UPDATE artists
    SET has_sc_account = EXISTS (SELECT 1
                                 FROM artist_sc_accounts AS account
                                 WHERE account.artist_id = OLD.artist_id
                                   AND (account.verified OR account.source = 'mb_resolve')
                                   AND account.role IN ('main', 'alt', 'demo'))
    WHERE id = OLD.artist_id;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS artist_sc_account_flag_trg ON artist_sc_accounts;
CREATE TRIGGER artist_sc_account_flag_trg
    AFTER INSERT OR UPDATE OF role, source, verified OR DELETE
    ON artist_sc_accounts
    FOR EACH ROW
EXECUTE FUNCTION artist_sc_account_flag();

UPDATE artists AS artist
SET has_sc_account = false
WHERE artist.has_sc_account
  AND NOT EXISTS (SELECT 1
                  FROM artist_sc_accounts AS account
                  WHERE account.artist_id = artist.id
                    AND (account.verified OR account.source = 'mb_resolve')
                    AND account.role IN ('main', 'alt', 'demo'));

CREATE INDEX IF NOT EXISTS track_artists_walker_revalidation_idx
    ON track_artists (track_id)
    WHERE source = 'walker';

CREATE TABLE IF NOT EXISTS artist_attribution_revalidation
(
    singleton               boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    walker_completed_at     timestamptz,
    identity_cursor_artist  uuid,
    identity_cursor_account text,
    identity_completed_at   timestamptz,
    credits_removed         bigint NOT NULL DEFAULT 0,
    tracks_reset            bigint NOT NULL DEFAULT 0,
    updated_at              timestamptz,
    CONSTRAINT artist_attribution_revalidation_cursor_pairing
        CHECK ((identity_cursor_artist IS NULL) = (identity_cursor_account IS NULL))
);

INSERT INTO artist_attribution_revalidation (singleton)
VALUES (true)
ON CONFLICT (singleton) DO NOTHING;
