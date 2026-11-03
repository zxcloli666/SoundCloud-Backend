use backend_contracts::admin_catalog::{
    AddCredit, AdminCommand, DetachAccountTracks, DetachArtist, SetAlbum, SetPrimaryArtist,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{artist, blocks, credit, credits, primary, send, track};

async fn block(pool: &PgPool, track: Uuid, artist: Uuid) {
    sqlx::query("INSERT INTO track_artist_blocks (track_id, artist_id) VALUES ($1, $2)")
        .bind(track)
        .bind(artist)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn setting_the_primary_artist_replaces_the_primary_credit_and_lifts_the_block(pool: PgPool) {
    let old = artist(&pool, "Old").await;
    let new = artist(&pool, "New").await;
    let track = track(&pool, None).await;
    credit(&pool, track, old, "primary").await;
    credit(&pool, track, old, "featured").await;
    block(&pool, track, new).await;

    let reply = send(
        &pool,
        AdminCommand::TrackSetPrimaryArtist {
            track_id: track,
            body: SetPrimaryArtist { artist_id: new },
        },
    )
    .await;

    assert_eq!(reply.status, 200);
    assert_eq!(primary(&pool, track).await, Some(new));
    assert_eq!(
        credits(&pool, track).await,
        vec![(old, "featured".to_owned()), (new, "primary".to_owned())]
    );
    assert_eq!(blocks(&pool, track).await, 0);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn an_unknown_artist_or_track_changes_nothing(pool: PgPool) {
    let known = artist(&pool, "Known").await;
    let track = track(&pool, None).await;
    credit(&pool, track, known, "primary").await;

    let unknown_artist = send(
        &pool,
        AdminCommand::TrackSetPrimaryArtist {
            track_id: track,
            body: SetPrimaryArtist {
                artist_id: Uuid::new_v4(),
            },
        },
    )
    .await;
    let unknown_track = send(
        &pool,
        AdminCommand::TrackSetPrimaryArtist {
            track_id: Uuid::new_v4(),
            body: SetPrimaryArtist { artist_id: known },
        },
    )
    .await;

    assert_eq!(unknown_artist.status, 400);
    assert_eq!(unknown_track.status, 404);
    assert_eq!(
        credits(&pool, track).await,
        vec![(known, "primary".to_owned())]
    );
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn a_primary_credit_displaces_the_other_primary(pool: PgPool) {
    let first = artist(&pool, "First").await;
    let second = artist(&pool, "Second").await;
    let track = track(&pool, None).await;
    credit(&pool, track, first, "primary").await;

    let reply = send(
        &pool,
        AdminCommand::TrackAddCredit {
            track_id: track,
            body: AddCredit {
                artist_id: second,
                role: "primary".to_owned(),
                position: None,
            },
        },
    )
    .await;

    assert_eq!(reply.body["role"], "primary");
    assert_eq!(
        credits(&pool, track).await,
        vec![(second, "primary".to_owned())]
    );
    assert_eq!(primary(&pool, track).await, Some(second));
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn the_feature_alias_is_stored_as_featured_and_a_bad_role_is_refused(pool: PgPool) {
    let artist = artist(&pool, "Guest").await;
    let track = track(&pool, None).await;
    let add = |role: &str| AdminCommand::TrackAddCredit {
        track_id: track,
        body: AddCredit {
            artist_id: artist,
            role: role.to_owned(),
            position: Some(2),
        },
    };

    let alias = send(&pool, add(" Feature ")).await;
    let bad = send(&pool, add("conductor")).await;

    assert_eq!(alias.body["role"], "featured");
    assert_eq!(bad.status, 400);
    assert_eq!(
        credits(&pool, track).await,
        vec![(artist, "featured".to_owned())]
    );
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn removing_the_primary_credit_clears_the_track_primary(pool: PgPool) {
    let artist = artist(&pool, "Lead").await;
    let track = track(&pool, None).await;
    send(
        &pool,
        AdminCommand::TrackSetPrimaryArtist {
            track_id: track,
            body: SetPrimaryArtist { artist_id: artist },
        },
    )
    .await;

    let reply = send(
        &pool,
        AdminCommand::TrackRemoveCredit {
            track_id: track,
            artist_id: artist,
            role: "primary".to_owned(),
        },
    )
    .await;

    assert_eq!(reply.body["removed"], 1);
    assert_eq!(primary(&pool, track).await, None);
    assert!(credits(&pool, track).await.is_empty());
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn detaching_an_artist_blocks_it_and_unblocking_lifts_the_block(pool: PgPool) {
    let artist = artist(&pool, "Wrong").await;
    let track = track(&pool, None).await;
    credit(&pool, track, artist, "primary").await;
    sqlx::query("UPDATE tracks SET primary_artist_id = $1 WHERE id = $2")
        .bind(artist)
        .bind(track)
        .execute(&pool)
        .await
        .unwrap();
    let detach = AdminCommand::TrackDetachArtist {
        track_id: track,
        body: DetachArtist {
            artist_id: artist,
            note: Some("not him".to_owned()),
        },
    };

    assert_eq!(send(&pool, detach.clone()).await.status, 200);
    assert_eq!(send(&pool, detach).await.status, 200);

    assert_eq!(blocks(&pool, track).await, 1);
    assert_eq!(primary(&pool, track).await, None);
    assert!(credits(&pool, track).await.is_empty());

    let unblocked = send(
        &pool,
        AdminCommand::TrackUnblockArtist {
            track_id: track,
            artist_id: artist,
        },
    )
    .await;
    assert_eq!(unblocked.body["removed"], 1);
    assert_eq!(blocks(&pool, track).await, 0);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn detaching_an_unknown_track_is_not_found(pool: PgPool) {
    let artist = artist(&pool, "Nobody").await;
    let reply = send(
        &pool,
        AdminCommand::TrackDetachArtist {
            track_id: Uuid::new_v4(),
            body: DetachArtist {
                artist_id: artist,
                note: None,
            },
        },
    )
    .await;
    assert_eq!(reply.status, 404);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn an_account_detach_blocks_only_that_uploaders_tracks(pool: PgPool) {
    let artist = artist(&pool, "Claimed").await;
    let theirs = track(&pool, Some("111")).await;
    let other = track(&pool, Some("222")).await;
    for track in [theirs, other] {
        credit(&pool, track, artist, "primary").await;
        sqlx::query("UPDATE tracks SET primary_artist_id = $1 WHERE id = $2")
            .bind(artist)
            .bind(track)
            .execute(&pool)
            .await
            .unwrap();
    }

    let reply = send(
        &pool,
        AdminCommand::ScAccountDetachTracks {
            artist_id: artist,
            sc_user_id: "111".to_owned(),
            body: DetachAccountTracks { note: None },
        },
    )
    .await;

    assert_eq!(reply.body["detached_tracks"], 1);
    assert_eq!(blocks(&pool, theirs).await, 1);
    assert_eq!(blocks(&pool, other).await, 0);
    assert_eq!(primary(&pool, theirs).await, None);
    assert_eq!(primary(&pool, other).await, Some(artist));
    assert!(credits(&pool, theirs).await.is_empty());
    assert_eq!(credits(&pool, other).await.len(), 1);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn an_album_assignment_moves_the_track_between_albums(pool: PgPool) {
    let track = track(&pool, None).await;
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    for (id, title) in [(first, "First"), (second, "Second")] {
        sqlx::query(
            "INSERT INTO albums (id, title, normalized_title, source) VALUES ($1, $2, lower($2), 'genius')",
        )
        .bind(id)
        .bind(title)
        .execute(&pool)
        .await
        .unwrap();
    }
    let assign = |album_id| AdminCommand::TrackSetAlbum {
        track_id: track,
        body: SetAlbum { album_id },
    };
    let slots = || async {
        sqlx::query_scalar::<_, Uuid>("SELECT album_id FROM album_tracks WHERE track_id = $1")
            .bind(track)
            .fetch_all(&pool)
            .await
            .unwrap()
    };

    send(&pool, assign(Some(first))).await;
    assert_eq!(slots().await, vec![first]);
    send(&pool, assign(Some(second))).await;
    assert_eq!(slots().await, vec![second]);
    let cleared = send(&pool, assign(None)).await;
    assert_eq!(cleared.status, 200);
    assert!(slots().await.is_empty());

    let unknown = send(&pool, assign(Some(Uuid::new_v4()))).await;
    assert_eq!(unknown.status, 400);
}
