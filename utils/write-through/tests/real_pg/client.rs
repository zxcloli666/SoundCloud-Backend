use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;
use serde_json::{Value, json};
use sqlx::postgres::PgDatabaseError;
use write_through as wt;
use wt::{Kind, Raced};

use crate::db::{self, hashes};
use crate::harness::Pair;

pub const TRACK: &str = "INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms)
    VALUES ('00000000-0000-0000-0000-0000000000a1', 'soundcloud:tracks:1', '1', 'Old', 'old', 1000)";
const DEAD_SYNC: &str =
    "INSERT INTO public.sync_queue (id, user_id, action_type, target_urn, dead, retry_count)
    VALUES ('00000000-0000-0000-0000-0000000000b1', 'u1', 'like', 'soundcloud:tracks:1', true, 7)";

#[derive(Debug, Serialize)]
struct Title {
    urn: &'static str,
    title: &'static str,
}

impl Kind for Title {
    const NAME: &'static str = "test.title";
    type Output = Value;
}

#[derive(Debug, Serialize)]
struct Playlist {
    urn: &'static str,
    sc_id: &'static str,
    title: &'static str,
    observed_at: &'static str,
}

#[derive(Debug, PartialEq, serde::Deserialize)]
struct Observed {
    was_new: bool,
}

impl Kind for Playlist {
    const NAME: &'static str = "test.playlist";
    type Output = Observed;
}

#[derive(Debug, Serialize)]
struct Revive {
    id: &'static str,
}

impl Kind for Revive {
    const NAME: &'static str = "test.revive";
    type Output = Option<Value>;
}

#[derive(Debug, Serialize)]
struct Stamp {
    urn: &'static str,
}

impl Kind for Stamp {
    const NAME: &'static str = "test.stamp";
    type Output = ();
}

#[derive(Debug, Serialize)]
struct Nameless;

impl Kind for Nameless {
    const NAME: &'static str = "test.nope";
    type Output = ();
}

#[derive(Debug, Serialize)]
struct Array;

impl Kind for Array {
    const NAME: &'static str = "test.array";
    type Output = ();
}

const NEW_TITLE: Title = Title {
    urn: "soundcloud:tracks:1",
    title: "New",
};

fn send<T: Send>(value: T) -> T {
    value
}

fn code(error: &sqlx::Error) -> String {
    error
        .as_database_error()
        .and_then(|e| e.try_downcast_ref::<PgDatabaseError>())
        .map(|e| format!("{} {}", e.code(), e.message()))
        .unwrap_or_else(|| error.to_string())
}

#[tokio::test]
async fn optimistic_kind_gives_equal_rows_and_output_on_api_and_core() {
    let pair = Pair::new(TRACK).await;
    let out = pair
        .run(&NEW_TITLE, &["lyrics_lookup_state"])
        .await
        .assert_same();
    assert_eq!(
        out,
        json!({"id": "00000000-0000-0000-0000-0000000000a1", "title": "New"})
    );
    let titles: (String, String) = sqlx::query_as(
        "SELECT (SELECT title FROM public.tracks WHERE urn = 'soundcloud:tracks:1'),
                (SELECT input_title FROM public.lyrics_lookup_state WHERE sc_track_id = '1')",
    )
    .fetch_one(&pair.core)
    .await
    .unwrap();
    assert_eq!(titles, ("New".to_owned(), "New".to_owned()));
    pair.finish().await;
}

#[tokio::test]
async fn derived_rows_are_core_only_and_show_up_as_a_difference() {
    let pair = Pair::new(TRACK).await;
    let run = pair.run(&NEW_TITLE, &[]).await;
    assert_eq!(
        run.diverged,
        [
            "api and core differ: lyrics_lookup_state",
            "core local run and dispatch differ: lyrics_lookup_state"
        ]
    );
    pair.finish().await;
}

#[tokio::test]
async fn output_decodes_into_the_kind_type_and_follows_the_rows() {
    let pair = Pair::new("").await;
    let first = Playlist {
        urn: "soundcloud:playlists:7",
        sc_id: "7",
        title: "One",
        observed_at: "2026-10-30T10:00:00Z",
    };
    let second = Playlist {
        title: "Two",
        observed_at: "2026-10-30T11:00:00Z",
        ..first
    };
    assert_eq!(
        pair.run(&first, &[]).await.assert_same(),
        json!({"was_new": true})
    );
    assert_eq!(
        pair.run(&second, &[]).await.assert_same(),
        json!({"was_new": false})
    );
    let typed = send(wt::retry(|| async {
        let mut tx = pair.core.begin().await?;
        let output = wt::call(&mut tx, &second).await?;
        tx.commit().await.map(|()| output)
    }))
    .await
    .unwrap();
    assert_eq!(typed, Observed { was_new: false });
    pair.finish().await;
}

#[tokio::test]
async fn core_runs_the_kind_in_place_and_the_api_ships_it() {
    let pair = Pair::new(TRACK).await;
    let counts = "SELECT (SELECT count(*) FROM public.cmd_outbox), (SELECT count(*) FROM public.cmd_pending)";
    let mut core = pair.core.begin().await.unwrap();
    wt::call(&mut core, &NEW_TITLE).await.unwrap();
    let on_core: (i64, i64) = sqlx::query_as(counts).fetch_one(&mut *core).await.unwrap();
    core.commit().await.unwrap();
    let mut api = pair.api.begin().await.unwrap();
    wt::call(&mut api, &NEW_TITLE).await.unwrap();
    let on_api: (i64, i64) = sqlx::query_as(counts).fetch_one(&mut *api).await.unwrap();
    let touched: Value = sqlx::query_scalar("SELECT touched FROM public.cmd_outbox")
        .fetch_one(&mut *api)
        .await
        .unwrap();
    api.commit().await.unwrap();
    assert_eq!(on_core, (0, 0));
    assert_eq!(on_api, (1, 1));
    assert_eq!(
        touched[0]["pk"],
        json!({"id": "00000000-0000-0000-0000-0000000000a1"})
    );
    assert_eq!(touched[0]["hash"].as_str().map(str::len), Some(32));
    pair.finish().await;
}

#[tokio::test]
async fn non_optimistic_kind_ships_from_the_api_without_running() {
    let pair = Pair::new(DEAD_SYNC).await;
    let run = pair
        .run(
            &Revive {
                id: "00000000-0000-0000-0000-0000000000b1",
            },
            &[],
        )
        .await;
    assert!(run.diverged.is_empty(), "{:#?}", run.diverged);
    assert_eq!(run.api, Value::Null);
    assert_eq!(run.core, json!({"revived": 1}));
    let revived: bool =
        sqlx::query_scalar("SELECT NOT dead AND retry_count = 0 FROM public.sync_queue")
            .fetch_one(&pair.core)
            .await
            .unwrap();
    let untouched: bool =
        sqlx::query_scalar("SELECT dead AND retry_count = 7 FROM public.sync_queue")
            .fetch_one(&pair.api)
            .await
            .unwrap();
    assert!(revived && untouched);
    pair.finish().await;
}

#[tokio::test]
async fn a_nondeterministic_kind_is_caught_by_the_hash() {
    let pair = Pair::new(TRACK).await;
    let run = pair
        .run(
            &Stamp {
                urn: "soundcloud:tracks:1",
            },
            &["lyrics_lookup_state"],
        )
        .await;
    assert!(
        run.diverged
            .contains(&"api and core differ: tracks".to_owned()),
        "{:#?}",
        run.diverged
    );
    assert!(
        run.diverged
            .contains(&"core local run and dispatch differ: tracks".to_owned()),
        "{:#?}",
        run.diverged
    );
    assert!(
        run.diverged
            .iter()
            .any(|d| d.starts_with("dispatch on core:") && d.contains("reasserted: 1")),
        "{:#?}",
        run.diverged
    );
    pair.finish().await;
}

#[tokio::test]
async fn refused_kinds_roll_back_and_are_not_retried() {
    let pair = Pair::new("").await;
    let before = hashes(&pair.api).await;
    let attempts = AtomicUsize::new(0);
    let refuse = async |kind| {
        wt::retry(|| async {
            attempts.fetch_add(1, Ordering::SeqCst);
            let mut tx = pair.api.begin().await?;
            match kind {
                "unknown" => wt::call(&mut tx, &Nameless).await?,
                _ => wt::call(&mut tx, &Array).await?,
            }
            tx.commit().await
        })
        .await
        .unwrap_err()
    };
    let unknown = refuse("unknown").await;
    let array = refuse("array").await;
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(code(&unknown), "22023 scd_wt: unknown kind test.nope");
    assert!(
        code(&array).starts_with("22023 scd_wt: kind test.array returned array"),
        "{}",
        code(&array)
    );
    assert!(!unknown.raced() && !array.raced());
    let outbox: i64 = sqlx::query_scalar("SELECT count(*) FROM public.cmd_outbox")
        .fetch_one(&pair.api)
        .await
        .unwrap();
    assert_eq!(outbox, 0);
    assert_eq!(hashes(&pair.api).await, before);
    pair.finish().await;
}

#[tokio::test]
async fn tokio_postgres_client_gives_the_same_output_and_rows() {
    let pair = Pair::new("").await;
    let args = Playlist {
        urn: "soundcloud:playlists:9",
        sc_id: "9",
        title: "Nine",
        observed_at: "2026-10-30T10:00:00Z",
    };
    let mut api = db::tokio_client(&pair.api_db).await;
    let mut core = db::tokio_client(&pair.core_db).await;
    let mut on = Vec::new();
    for client in [&mut api, &mut core] {
        let tx = client.transaction().await.unwrap();
        on.push(wt::tokio_pg::call(&tx, &args).await.unwrap());
        tx.commit().await.unwrap();
    }
    let (on_api, on_core) = (&on[0], &on[1]);
    assert_eq!(on_api, &Observed { was_new: true });
    assert_eq!(on_core, on_api);
    let outbox = api
        .query_one("SELECT kind, args FROM public.cmd_outbox", &[])
        .await
        .unwrap();
    assert_eq!(outbox.get::<_, String>(0), "test.playlist");
    assert_eq!(
        outbox.get::<_, Value>(1),
        serde_json::to_value(&args).unwrap()
    );
    assert_eq!(hashes(&pair.api).await, hashes(&pair.core).await);
    let null = core
        .query_one("SELECT count(*) FROM public.cmd_outbox", &[])
        .await
        .unwrap();
    assert_eq!(null.get::<_, i64>(0), 0);
    pair.finish().await;
}
