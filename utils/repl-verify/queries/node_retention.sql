SELECT batch.deleted AS "deleted!",
       batch.redacted AS "redacted!",
       batch.held,
       public.scd_retention_reason(batch.held) AS reason
FROM public.scd_node_retention_batch($1, batch => $2) AS batch
