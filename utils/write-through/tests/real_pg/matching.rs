use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;
use write_through as wt;
use wt::{AlbumSlot, MatchAlbumTrack, MatchGeniusSong, MatchScAccount, MatchWantedLink};

use crate::harness::Pair;

const ARTIST_ID: &str = "0b7e5c3a-1d2f-4a6b-8c9d-3e4f5a6b7c8d";
const TRACK_ID: &str = "6f1c2b9e-4d7a-4c3e-9b8f-2a5d7e1c0b94";
const WANTED_ID: &str = "4a3b2c1d-9e8f-4b7a-8c6d-5e4f3a2b1c0d";
const ALBUM_A: &str = "7c6b5a49-3827-4615-9a4b-3c2d1e0f9a8b";
const ALBUM_B: &str = "8d7c6b5a-4938-4726-8b5c-4d3e2f1a0b9c";
const CATALOG: &str = "INSERT INTO public.artists (id, name, normalized_name, source, created_at, updated_at)
    VALUES ('0b7e5c3a-1d2f-4a6b-8c9d-3e4f5a6b7c8d', 'Kavinsky Fan', 'kavinsky fan', 'genius',
        '2026-03-01 10:00:00+00', '2026-03-01 10:00:00+00');
    INSERT INTO public.tracks (id, sc_track_id, urn, title, title_normalized, duration_ms, created_at, updated_at)
    VALUES ('6f1c2b9e-4d7a-4c3e-9b8f-2a5d7e1c0b94', '1874620031', 'soundcloud:tracks:1874620031',
        'Night Drive', 'night drive', 213000, '2026-01-15 12:00:05+00', '2026-10-28 08:00:00+00');
    INSERT INTO public.albums (id, title, normalized_title, source, created_at, updated_at)
    VALUES ('7c6b5a49-3827-4615-9a4b-3c2d1e0f9a8b', 'Outrun', 'outrun', 'genius', '2026-03-01 10:00:00+00', '2026-03-01 10:00:00+00'),
           ('8d7c6b5a-4938-4726-8b5c-4d3e2f1a0b9c', 'Outrun Deluxe', 'outrun deluxe', 'genius', '2026-03-01 10:00:00+00', '2026-03-01 10:00:00+00');
    INSERT INTO public.wanted_tracks (id, title, normalized_title, source, external_id, status, updated_at)
    VALUES ('4a3b2c1d-9e8f-4b7a-8c6d-5e4f3a2b1c0d', 'Night Drive', 'night drive', 'genius_crawl', '5521337',
        'pending', '2026-10-01 10:00:00+00')";

fn at(stamp: &str) -> DateTime<Utc> {
    stamp.parse().unwrap()
}

fn id(text: &str) -> Uuid {
    text.parse().unwrap()
}

#[tokio::test]
async fn a_matched_account_backfills_the_artist_and_can_be_deleted() {
    let pair = Pair::new(CATALOG).await;
    let upsert = MatchScAccount::Upsert {
        artist_id: id(ARTIST_ID),
        sc_user_id: "204411307".to_owned(),
        role: "main".to_owned(),
        source: "genius".to_owned(),
        verified: true,
        at: at("2026-10-30T10:00:00Z"),
    };
    assert_eq!(pair.run(&upsert, &[]).await.assert_same(), json!(true));
    let artist: (Option<String>, bool, DateTime<Utc>) =
        sqlx::query_as("SELECT sc_user_id, has_sc_account, updated_at FROM public.artists")
            .fetch_one(&pair.api)
            .await
            .unwrap();
    assert_eq!(
        artist,
        (
            Some("204411307".to_owned()),
            true,
            at("2026-10-30T10:00:00Z")
        )
    );
    let delete = MatchScAccount::Delete {
        artist_id: id(ARTIST_ID),
        sc_user_id: "204411307".to_owned(),
    };
    assert_eq!(pair.run(&delete, &[]).await.assert_same(), json!(true));
    assert_eq!(pair.run(&delete, &[]).await.assert_same(), json!(false));
    pair.finish().await;
}

#[tokio::test]
async fn a_manual_account_keeps_its_source_and_a_verified_role_wins() {
    let pair = Pair::new(&format!(
        "{CATALOG}; INSERT INTO public.artist_sc_accounts (artist_id, sc_user_id, role, source, verified, created_at, updated_at)
         VALUES ('{ARTIST_ID}', '204411307', 'alt', 'manual', false, '2026-04-01 10:00:00+00', '2026-04-01 10:00:00+00')"
    ))
    .await;
    let upsert = MatchScAccount::Upsert {
        artist_id: id(ARTIST_ID),
        sc_user_id: "204411307".to_owned(),
        role: "main".to_owned(),
        source: "genius".to_owned(),
        verified: true,
        at: at("2026-10-30T10:00:00Z"),
    };
    pair.run(&upsert, &[]).await.assert_same();
    let account: (String, String, bool, DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
        "SELECT role, source, verified, created_at, updated_at FROM public.artist_sc_accounts",
    )
    .fetch_one(&pair.core)
    .await
    .unwrap();
    assert_eq!(
        account,
        (
            "main".to_owned(),
            "manual".to_owned(),
            true,
            at("2026-04-01T10:00:00Z"),
            at("2026-10-30T10:00:00Z")
        )
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_wanted_track_links_to_the_winner_of_a_superseded_track() {
    let pair = Pair::new(&format!(
        "{CATALOG}; INSERT INTO public.tracks (id, sc_track_id, urn, title, title_normalized, duration_ms, superseded_by)
         VALUES ('9a8b7c6d-5e4f-4a3b-9c2d-1e0f9a8b7c6d', '1874620032', 'soundcloud:tracks:1874620032',
             'Night Drive', 'night drive', 213000, '{TRACK_ID}')"
    ))
    .await;
    let link = MatchWantedLink {
        wanted_id: id(WANTED_ID),
        sc_track_id: "1874620032".to_owned(),
        at: at("2026-10-30T10:00:00Z"),
    };
    assert_eq!(pair.run(&link, &[]).await.assert_same(), json!(TRACK_ID));
    let missing = MatchWantedLink {
        sc_track_id: "1".to_owned(),
        ..link
    };
    assert_eq!(pair.run(&missing, &[]).await.assert_same(), Value::Null);
    let wanted: (Option<Uuid>, String, DateTime<Utc>) =
        sqlx::query_as("SELECT track_id, status, updated_at FROM public.wanted_tracks")
            .fetch_one(&pair.core)
            .await
            .unwrap();
    assert_eq!(
        wanted,
        (
            Some(id(TRACK_ID)),
            "linked".to_owned(),
            at("2026-10-30T10:00:00Z")
        )
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_genius_song_attaches_once() {
    let pair = Pair::new(CATALOG).await;
    let attach = MatchGeniusSong {
        track_id: id(TRACK_ID),
        genius_song_id: 5521337,
    };
    assert_eq!(
        pair.run(&attach, &["lyrics_lookup_state"])
            .await
            .assert_same(),
        json!("1874620031")
    );
    let again = MatchGeniusSong {
        genius_song_id: 1,
        ..attach
    };
    assert_eq!(
        pair.run(&again, &["lyrics_lookup_state"])
            .await
            .assert_same(),
        Value::Null
    );
    pair.finish().await;
}

#[tokio::test]
async fn album_slots_fill_the_track_once_and_link_every_album() {
    let pair = Pair::new(CATALOG).await;
    let albums = MatchAlbumTrack {
        track_id: id(TRACK_ID),
        albums: vec![
            AlbumSlot {
                album_id: id(ALBUM_A),
                position: 3,
            },
            AlbumSlot {
                album_id: id(ALBUM_B),
                position: 7,
            },
        ],
    };
    assert_eq!(
        pair.run(&albums, &["lyrics_lookup_state"])
            .await
            .assert_same(),
        Value::Null
    );
    let track: (Option<Uuid>, Option<i16>) =
        sqlx::query_as("SELECT album_id, album_position FROM public.tracks")
            .fetch_one(&pair.core)
            .await
            .unwrap();
    assert_eq!(track, (Some(id(ALBUM_A)), Some(3)));
    let links: Vec<(Uuid, Option<i16>)> =
        sqlx::query_as("SELECT album_id, position FROM public.album_tracks ORDER BY position")
            .fetch_all(&pair.api)
            .await
            .unwrap();
    assert_eq!(links, [(id(ALBUM_A), Some(3)), (id(ALBUM_B), Some(7))]);
    pair.finish().await;
}

#[tokio::test]
async fn an_identity_verified_for_another_artist_is_refused_with_its_constraint() {
    let pair = Pair::new(&format!(
        "{CATALOG}; INSERT INTO public.artists (id, name, normalized_name, source)
         VALUES ('1c8f6d4b-2e3a-4b7c-9d0e-4f5a6b7c8d9e', 'Other', 'other', 'genius');
         INSERT INTO public.artist_sc_accounts (artist_id, sc_user_id, role, source, verified)
         VALUES ('1c8f6d4b-2e3a-4b7c-9d0e-4f5a6b7c8d9e', '204411307', 'main', 'manual', true)"
    ))
    .await;
    let claim = MatchScAccount::Upsert {
        artist_id: id(ARTIST_ID),
        sc_user_id: "204411307".to_owned(),
        role: "main".to_owned(),
        source: "genius".to_owned(),
        verified: true,
        at: at("2026-10-30T10:00:00Z"),
    };
    let error = wt::retry(|| async {
        let mut tx = pair.api.begin().await?;
        wt::call(&mut tx, &claim).await?;
        tx.commit().await
    })
    .await
    .unwrap_err();
    assert!(!wt::Raced::raced(&error));
    assert_eq!(
        error.as_database_error().and_then(|e| e.constraint()),
        Some("artist_sc_accounts_verified_identity_uq")
    );
    let outbox: i64 = sqlx::query_scalar("SELECT count(*) FROM public.cmd_outbox")
        .fetch_one(&pair.api)
        .await
        .unwrap();
    assert_eq!(outbox, 0);
    pair.finish().await;
}
