SELECT p.state AS "state!", p.probes AS "probes!", p.rel, p.wrapped AS "wrapped!"
FROM public.scd_verify_probe($1, $2) AS p
