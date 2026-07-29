-- Волатильные SC-счётчики (play/likes/reposts/followers) дрейфуют непрерывно.
-- Точное сравнение в ON CONFLICT ... WHERE переписывало строку на каждый
-- sighting: на 34-ГБ `tracks` с ~20 индексами каждая такая перезапись —
-- не-HOT update (индексирован и `play_count_sc`, и `sc_synced_at`), т.е. новый
-- heap-tuple + 20 индексных записей + full-page writes в WAL.
-- Порог дрейфа: >5% или минимум 5 единиц. Все потребители этих колонок —
-- ранжирование (ORDER BY popularity, ln1p-бонус в рекомендациях), им
-- достаточно приближения.
CREATE OR REPLACE FUNCTION sc_counter_drifted(old bigint, new bigint)
    RETURNS boolean
    LANGUAGE sql
    IMMUTABLE
    PARALLEL SAFE
AS
$$
SELECT new IS NOT NULL
   AND (old IS NULL OR abs(new - old) > GREATEST(5, abs(old) / 20))
$$;

-- Свежесть per-user коллекции. Раньше её держал MAX(synced_at) по всем
-- строкам зеркала — из-за этого refresh обязан был проштамповать synced_at на
-- КАЖДОЙ строке (5k лайков = 5k UPDATE'ов на refresh), а чтение сканировало
-- все строки юзера на каждый запрос. Маркер — одна строка на (user, kind).
CREATE TABLE IF NOT EXISTS "user_collection_sync"
(
    "user_id"    text        NOT NULL,
    "collection" text        NOT NULL,
    "synced_at"  timestamptz NOT NULL,
    PRIMARY KEY ("user_id", "collection")
);
