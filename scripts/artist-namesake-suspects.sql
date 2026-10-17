WITH trusted AS (SELECT account.artist_id,
                        array_agg(account.sc_user_id ORDER BY account.sc_user_id) AS accounts
                 FROM artist_sc_accounts AS account
                 WHERE account.verified
                    OR account.source IN ('mb_resolve', 'manual')
                 GROUP BY account.artist_id),
     name_matched AS (SELECT account.artist_id,
                             array_agg(account.sc_user_id || ':' || account.role
                                       ORDER BY account.sc_user_id) AS accounts,
                             bool_or(account.role IN ('main', 'demo'))  AS shown
                      FROM artist_sc_accounts AS account
                      WHERE account.source = 'auto_match'
                        AND NOT account.verified
                      GROUP BY account.artist_id),
     foreign_avatar AS (SELECT DISTINCT account.artist_id
                        FROM artist_sc_accounts AS account
                                 JOIN artists AS artist ON artist.id = account.artist_id
                        WHERE account.source IN ('auto_match', 'reupload_pattern')
                          AND NOT account.verified
                          AND artist.avatar_url IS NOT NULL
                          AND EXISTS (SELECT 1
                                      FROM tracks AS track
                                      WHERE track.uploader_sc_user_id = account.sc_user_id
                                        AND replace(track.uploader_avatar_url, '-large.', '-t500x500.') =
                                            artist.avatar_url)),
     foreign_credits AS (SELECT credit.artist_id,
                                count(*)                                    AS tracks,
                                count(DISTINCT track.uploader_sc_user_id) AS uploaders,
                                (array_agg(DISTINCT track.uploader_sc_user_id || ':' ||
                                                    coalesce(track.uploader_username, '')))[1:8] AS sample
                         FROM trusted
                                  JOIN artists AS artist ON artist.id = trusted.artist_id
                                  JOIN track_artists AS credit
                                       ON credit.artist_id = trusted.artist_id
                                           AND credit.role = 'primary'
                                  JOIN tracks AS track ON track.id = credit.track_id
                         WHERE artist.merged_into IS NULL
                           AND credit.evidence IN ('uploader_name', 'title_heuristic', 'ai_inference',
                                                   'metadata_field', 'unattributed')
                           AND (credit.evidence = 'uploader_name' OR char_length(artist.normalized_name) < 4)
                           AND NOT track.is_cover
                           AND track.superseded_by IS NULL
                           AND track.uploader_sc_user_id IS NOT NULL
                           AND NOT track.uploader_sc_user_id = ANY (trusted.accounts)
                         GROUP BY credit.artist_id),
     suspects AS (SELECT artist.id,
                         artist.name,
                         artist.normalized_name,
                         artist.sc_user_id,
                         artist.avatar_url,
                         trusted.accounts                       AS trusted_accounts,
                         name_matched.accounts                  AS auto_match_accounts,
                         coalesce(foreign_credits.tracks, 0)    AS foreign_primary_tracks,
                         coalesce(foreign_credits.uploaders, 0) AS foreign_uploaders,
                         foreign_credits.sample                 AS foreign_uploader_sample,
                         array_remove(ARRAY [
                             CASE
                                 WHEN name_matched.shown AND trusted.artist_id IS NOT NULL
                                     THEN 'auto_match_main_beside_trusted' END,
                             CASE
                                 WHEN name_matched.artist_id IS NOT NULL AND char_length(artist.normalized_name) < 4
                                     THEN 'auto_match_on_short_name' END,
                             CASE
                                 WHEN artist.sc_user_id IS NOT NULL AND NOT artist.sc_user_id = ANY (trusted.accounts)
                                     THEN 'link_not_trusted' END,
                             CASE WHEN foreign_avatar.artist_id IS NOT NULL THEN 'avatar_of_untrusted_account' END,
                             CASE WHEN foreign_credits.artist_id IS NOT NULL THEN 'foreign_name_only_primaries' END
                             ], NULL)                           AS reasons
                  FROM artists AS artist
                           LEFT JOIN trusted ON trusted.artist_id = artist.id
                           LEFT JOIN name_matched ON name_matched.artist_id = artist.id
                           LEFT JOIN foreign_avatar ON foreign_avatar.artist_id = artist.id
                           LEFT JOIN foreign_credits ON foreign_credits.artist_id = artist.id
                  WHERE artist.merged_into IS NULL
                    AND (trusted.artist_id IS NOT NULL
                      OR name_matched.artist_id IS NOT NULL
                      OR foreign_avatar.artist_id IS NOT NULL))
SELECT *
FROM suspects
WHERE cardinality(suspects.reasons) > 0
ORDER BY cardinality(suspects.reasons) DESC,
         suspects.foreign_primary_tracks DESC,
         suspects.normalized_name;
