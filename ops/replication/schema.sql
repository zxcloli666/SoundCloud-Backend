WITH rels AS (
    SELECT c.oid, c.relname
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p')
),
cols AS (
    SELECT a.attrelid AS oid,
           jsonb_agg(jsonb_build_object(
               'name', a.attname,
               'type', pg_catalog.format_type(a.atttypid, a.atttypmod),
               'notnull', a.attnotnull,
               'default', pg_catalog.pg_get_expr(d.adbin, d.adrelid),
               'generated', a.attgenerated <> ''
           ) ORDER BY a.attname) AS list
    FROM pg_catalog.pg_attribute a
    LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
    WHERE a.attnum > 0 AND NOT a.attisdropped AND a.attrelid IN (SELECT oid FROM rels)
    GROUP BY a.attrelid
),
idx AS (
    SELECT i.indrelid AS oid,
           jsonb_agg(jsonb_build_object(
               'name', ic.relname,
               'primary', i.indisprimary,
               'cols', (SELECT jsonb_agg(a.attname ORDER BY k.ord)
                        FROM unnest(i.indkey::int2[]) WITH ORDINALITY AS k(attnum, ord)
                        JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum),
               'expr', i.indexprs IS NOT NULL,
               'keys', (SELECT jsonb_agg(pg_catalog.pg_get_indexdef(i.indexrelid, k, true) ORDER BY k)
                        FROM generate_series(1, i.indnkeyatts) AS k),
               'where', pg_catalog.pg_get_expr(i.indpred, i.indrelid)
           ) ORDER BY ic.relname) AS list
    FROM pg_catalog.pg_index i
    JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid
    WHERE i.indisunique AND i.indrelid IN (SELECT oid FROM rels)
    GROUP BY i.indrelid
),
fks AS (
    SELECT c.conrelid AS oid,
           jsonb_agg(jsonb_build_object(
               'name', c.conname,
               'cols', (SELECT jsonb_agg(a.attname ORDER BY k.ord)
                        FROM unnest(c.conkey) WITH ORDINALITY AS k(attnum, ord)
                        JOIN pg_catalog.pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum),
               'ref_table', rc.relname,
               'ref_cols', (SELECT jsonb_agg(a.attname ORDER BY k.ord)
                            FROM unnest(c.confkey) WITH ORDINALITY AS k(attnum, ord)
                            JOIN pg_catalog.pg_attribute a ON a.attrelid = c.confrelid AND a.attnum = k.attnum),
               'on_delete', c.confdeltype
           ) ORDER BY c.conname) AS list
    FROM pg_catalog.pg_constraint c
    JOIN pg_catalog.pg_class rc ON rc.oid = c.confrelid
    WHERE c.contype = 'f' AND c.conrelid IN (SELECT oid FROM rels)
    GROUP BY c.conrelid
),
trg AS (
    SELECT t.tgrelid AS oid,
           jsonb_agg(jsonb_build_object(
               'name', t.tgname,
               'function', p.proname,
               'enabled', t.tgenabled,
               'gated', pg_catalog.pg_get_triggerdef(t.oid) LIKE '%scd_derived()%',
               'pinned', EXISTS (SELECT 1 FROM unnest(p.proconfig) AS c WHERE c LIKE 'search_path=%')
           ) || CASE WHEN t.tgname LIKE 'zz%' THEN '{}'::jsonb ELSE jsonb_build_object(
               'before', (t.tgtype & 2) <> 0,
               'source', p.prosrc
           ) END ORDER BY t.tgname) AS list
    FROM pg_catalog.pg_trigger t
    JOIN pg_catalog.pg_proc p ON p.oid = t.tgfoid
    WHERE NOT t.tgisinternal AND t.tgrelid IN (SELECT oid FROM rels)
    GROUP BY t.tgrelid
)
SELECT jsonb_pretty(jsonb_object_agg(r.relname, jsonb_build_object(
    'columns', coalesce(cols.list, '[]'::jsonb),
    'unique', coalesce(idx.list, '[]'::jsonb),
    'fks', coalesce(fks.list, '[]'::jsonb),
    'triggers', coalesce(trg.list, '[]'::jsonb)
)))
FROM rels r
LEFT JOIN cols ON cols.oid = r.oid
LEFT JOIN idx ON idx.oid = r.oid
LEFT JOIN fks ON fks.oid = r.oid
LEFT JOIN trg ON trg.oid = r.oid;
