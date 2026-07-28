-- Ops-БД: телеметрия рекомендаций. Пишется на отдаче, читается только
-- тренером/аналитикой — ни один serving-запрос от неё не зависит.
-- Переехало из core (`0005_recommendations_v2`), где `features` (вектор фич
-- LTR на каждый показанный трек) раздувал main и лился в main<->star.
--
-- Нумерация 9000+ не пересекается с core (0000+): оба набора могут делить
-- одну базу и один `_sqlx_migrations` (см. `db::CORE_MIGRATOR`).

CREATE TABLE IF NOT EXISTS "rec_impressions" (
    "id" bigserial PRIMARY KEY,
    "sc_user_id" text NOT NULL,
    "sc_track_id" text NOT NULL,
    "cluster_id" text NOT NULL,
    "source" varchar(16) NOT NULL,
    "position" smallint NOT NULL,
    "score" real,
    "features" jsonb,
    "shown_at" timestamp with time zone NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS "rec_impressions_user_shown_idx"
    ON "rec_impressions" ("sc_user_id", "shown_at" DESC);
CREATE INDEX IF NOT EXISTS "rec_impressions_track_idx"
    ON "rec_impressions" ("sc_track_id");
CREATE INDEX IF NOT EXISTS "rec_impressions_cluster_user_idx"
    ON "rec_impressions" ("sc_user_id", "cluster_id", "shown_at" DESC);

CREATE TABLE IF NOT EXISTS "rec_hard_negatives" (
    "id" bigserial PRIMARY KEY,
    "sc_user_id" text NOT NULL,
    "sc_track_id" text NOT NULL,
    "predicted_score" real NOT NULL,
    "position_pct" real NOT NULL,
    "detected_at" timestamp with time zone NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS "rec_hard_negatives_user_idx"
    ON "rec_hard_negatives" ("sc_user_id", "detected_at" DESC);
