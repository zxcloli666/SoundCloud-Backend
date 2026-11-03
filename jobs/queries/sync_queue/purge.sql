SELECT public.scd_purge_intent('sync_queue', jsonb_build_object('id', $1::uuid), $2) AS "purged!"
