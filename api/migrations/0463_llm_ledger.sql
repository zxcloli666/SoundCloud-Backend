SET LOCAL lock_timeout = '5s';

ALTER TABLE public.video_gen_spend RENAME TO llm_spend;
ALTER TABLE public.llm_spend RENAME CONSTRAINT video_gen_spend_usd_valid TO llm_spend_usd_valid;
ALTER INDEX public.video_gen_spend_pkey RENAME TO llm_spend_pkey;
ALTER TABLE public.llm_spend ADD COLUMN ladder text NOT NULL DEFAULT 'video_gen';
ALTER TABLE public.llm_spend ALTER COLUMN ladder DROP DEFAULT;
DROP INDEX public.video_gen_spend_period_idx;
CREATE INDEX llm_spend_ladder_period_idx ON public.llm_spend (ladder, period);

ALTER TABLE public.video_gen_pause RENAME TO llm_pause;
ALTER TABLE public.llm_pause ADD COLUMN ladder text NOT NULL DEFAULT 'video_gen';
ALTER TABLE public.llm_pause ALTER COLUMN ladder DROP DEFAULT;
ALTER TABLE public.llm_pause DROP CONSTRAINT video_gen_pause_single;
ALTER TABLE public.llm_pause DROP CONSTRAINT video_gen_pause_pkey;
ALTER TABLE public.llm_pause DROP COLUMN id;
ALTER TABLE public.llm_pause ADD CONSTRAINT llm_pause_pkey PRIMARY KEY (ladder);
ALTER TABLE public.llm_pause RENAME CONSTRAINT video_gen_pause_reason_valid TO llm_pause_reason_valid;

DROP TRIGGER IF EXISTS zz_guard ON public.llm_spend;
DROP TRIGGER IF EXISTS zz_guard ON public.llm_pause;
DROP FUNCTION IF EXISTS public.scd_row_hash_video_gen_spend(jsonb);
DROP FUNCTION IF EXISTS public.scd_row_json_video_gen_spend(jsonb);
DROP FUNCTION IF EXISTS public.scd_purge_video_gen_spend(jsonb);
DROP FUNCTION IF EXISTS public.scd_row_hash_video_gen_pause(jsonb);
DROP FUNCTION IF EXISTS public.scd_row_json_video_gen_pause(jsonb);
DROP FUNCTION IF EXISTS public.scd_purge_video_gen_pause(jsonb);
DROP SEQUENCE IF EXISTS public.scd_guard_video_gen_spend;
DROP SEQUENCE IF EXISTS public.scd_guard_video_gen_pause;
