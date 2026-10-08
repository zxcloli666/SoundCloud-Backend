SELECT COALESCE(
           (SELECT scanned_through FROM rec_hard_negative_sweep WHERE singleton),
           (now() AT TIME ZONE 'UTC') - $1::bigint * interval '1 second'
       ) AS "scanned_through!",
       COALESCE(
           (SELECT scanned_event_id FROM rec_hard_negative_sweep WHERE singleton),
           '00000000-0000-0000-0000-000000000000'::uuid
       ) AS "scanned_event_id!"
