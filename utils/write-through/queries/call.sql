SELECT coalesce(w.output, 'null') AS "output!: _" FROM public.scd_wt($1, $2) AS w
