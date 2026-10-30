SELECT batch.deleted AS "deleted!",
       batch.held
FROM public.scd_retention_batch($1, batch => $2) AS batch
