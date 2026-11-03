SET LOCAL lock_timeout = '5s';
ALTER TABLE public.repl_tombstones
    DROP CONSTRAINT repl_tombstones_pkey,
    ADD CONSTRAINT repl_tombstones_pkey PRIMARY KEY USING INDEX repl_tombstones_version_key;
