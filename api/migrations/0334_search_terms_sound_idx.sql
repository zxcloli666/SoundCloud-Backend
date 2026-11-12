-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS search_terms_sound_idx ON public.search_terms (search_sound(latin)) WHERE word !~ '[а-яёіїєґ]';
