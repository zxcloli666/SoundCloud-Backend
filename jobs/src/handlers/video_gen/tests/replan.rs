use backend_contracts::worker_contract::WorkerLane;
use serde_json::Value;
use sqlx::PgPool;

use super::super::director::Reshot;
use super::flow::{accept, keyframe, payload, redo, shot, waiting};
use super::{TRACK, bench, config, plan, reply, row};
use crate::config::VideoGenConfig;

fn reshot() -> Reshot {
    Reshot {
        keyframe: "she sits still by a rainy window".to_owned(),
        motion: "still camera, rain runs down the glass".to_owned(),
    }
}

async fn state(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT state FROM track_video_gen WHERE sc_track_id = $1")
        .bind(TRACK)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_shot_out_of_tries_is_planned_again_and_accepted_shots_stay(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, store, wire, _) = bench(&pool, config());
    director.plans(Ok(plan()));
    bench.dispatch().await.unwrap();
    for verdict in [accept(0), accept(0), accept(0)] {
        director.verdicts(Ok(verdict));
    }
    keyframe(&bench, &wire).await;
    bench.dispatch().await.unwrap();
    shot(&bench, &wire).await;
    bench.dispatch().await.unwrap();
    let first = state(&pool).await["accepted"][0]["key"].clone();
    keyframe(&bench, &wire).await;
    bench.dispatch().await.unwrap();
    for edit in ["slower", "still camera"] {
        director.verdicts(Ok(redo(edit)));
        shot(&bench, &wire).await;
        bench.dispatch().await.unwrap();
    }

    assert_eq!(row(&pool, TRACK).await.0, "plan");
    director.reshots(Ok(reshot()));
    bench.dispatch().await.unwrap();

    assert_eq!(
        *director.replans.lock().unwrap(),
        [(1, vec!["the face changed".to_owned(); 2])]
    );
    assert_eq!(wire.last().0, "video.keyframe.new");
    assert!(
        payload(&wire)["prompt"]
            .as_str()
            .unwrap()
            .starts_with("she sits still by a rainy window")
    );
    director.verdicts(Ok(accept(0)));
    director.verdicts(Ok(accept(0)));
    keyframe(&bench, &wire).await;
    bench.dispatch().await.unwrap();
    assert!(
        payload(&wire)["prompt"]
            .as_str()
            .unwrap()
            .starts_with("still camera, rain runs down the glass")
    );
    shot(&bench, &wire).await;
    bench.dispatch().await.unwrap();

    let assemble = payload(&wire);
    assert_eq!(wire.last().0, "video.assemble.new");
    assert_eq!(
        assemble["shots"][0]["url"],
        format!("https://storage.example/{}", first.as_str().unwrap())
    );
    assert_eq!(assemble["shots"].as_array().unwrap().len(), 2);
    let state = state(&pool).await;
    assert_eq!(state["passed"].as_array().unwrap().len(), 5);
    assert_eq!(state["passed"][1]["note"], "the lock holds");
    assert_eq!(
        (state["redos"].clone(), state["replans"].clone()),
        (2.into(), 0.into())
    );
    let kept: Vec<String> = store
        .keys()
        .into_iter()
        .filter(|key| key.starts_with("video-tmp/gen-rejected/"))
        .collect();
    assert_eq!(kept.len(), 7);
    reply(
        &bench,
        &wire,
        WorkerLane::VideoAssemble,
        &[("video.mp4", b"final-video")],
        512,
    )
    .await;
    assert_eq!(row(&pool, TRACK).await.0, "done");
    assert_eq!(store.keys().len(), 8);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_spent_track_budget_rejects_the_video(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, _, wire, _) = bench(
        &pool,
        VideoGenConfig {
            keyframe_tries: 10,
            track_redos: 3,
            ..config()
        },
    );
    director.plans(Ok(plan()));
    bench.dispatch().await.unwrap();

    for edit in ["fix the face", "keep the coat", "same hair"] {
        director.verdicts(Ok(redo(edit)));
        keyframe(&bench, &wire).await;
        bench.dispatch().await.unwrap();
    }

    let (stage, reason, _, _) = row(&pool, TRACK).await;
    assert_eq!(stage, "rejected");
    assert!(
        reason
            .unwrap()
            .starts_with("3 redos spent, the last on keyframe 1")
    );
    assert_eq!(wire.subjects().len(), 3);
    assert!(director.replans.lock().unwrap().is_empty());
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_shot_out_of_re_plans_rejects_the_video(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, _, wire, _) = bench(
        &pool,
        VideoGenConfig {
            keyframe_tries: 1,
            shot_replans: 1,
            ..config()
        },
    );
    director.plans(Ok(plan()));
    director.reshots(Ok(reshot()));
    bench.dispatch().await.unwrap();

    director.verdicts(Ok(redo("fix the face")));
    keyframe(&bench, &wire).await;
    bench.dispatch().await.unwrap();
    bench.dispatch().await.unwrap();
    director.verdicts(Ok(redo("keep the coat")));
    keyframe(&bench, &wire).await;
    bench.dispatch().await.unwrap();

    let (stage, reason, _, _) = row(&pool, TRACK).await;
    assert_eq!(stage, "rejected");
    assert!(
        reason
            .unwrap()
            .starts_with("keyframe 1 failed after 1 re-plans")
    );
    assert_eq!(director.replans.lock().unwrap().len(), 1);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_plan_with_the_wrong_shot_count_is_asked_again_at_once(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, _, wire, _) = bench(&pool, config());
    let mut short = plan();
    short.shots.truncate(1);
    director.plans(Ok(short));
    director.plans(Ok(plan()));

    bench.dispatch().await.unwrap();

    assert_eq!(
        *director.refusals.lock().unwrap(),
        [None, Some("1 shots, 2 were asked for".to_owned())]
    );
    let (stage, _, rejections, faults) = row(&pool, TRACK).await;
    assert_eq!((stage.as_str(), faults), ("keyframe", 0));
    assert_eq!(rejections.as_array().unwrap().len(), 1);
    assert_eq!(rejections[0]["look"], "plan");
    assert_eq!(wire.subjects(), ["video.keyframe.new"]);
}
