use chrono::{Duration, Utc};
use serde_json::{Value, json};

use super::twin::{Step, Twin};
use crate::{Observation, TrackPriority};

const SEEDED_TRACK: u64 = 1874620031;
const NEW_TRACK: u64 = 1874620099;
const ABSENT_TRACK: u64 = 1874620100;
const DELETED_TRACK: u64 = 1874620101;
const OWNER: u64 = 204411307;
const OTHER_USER: u64 = 1500300;
const THIRD_USER: u64 = 1500301;

fn seed() -> String {
    let synced = (Utc::now() - Duration::days(2)).to_rfc3339();
    format!(
        "INSERT INTO public.tracks (
             id, sc_track_id, urn, title, title_normalized, duration_ms, sharing, artwork_url, permalink_url,
             waveform_url, uploader_sc_user_id, uploader_urn, uploader_username, sc_created_at, sc_last_modified,
             sc_synced_at, sc_observation, index_priority, storage_priority, storage_state, index_state,
             play_count_sc, created_at, updated_at, enrich_next_run_at)
         VALUES ('6f1c2b9e-4d7a-4c3e-9b8f-2a5d7e1c0b94', '{SEEDED_TRACK}', 'soundcloud:tracks:{SEEDED_TRACK}',
             'Night Drive', 'night drive', 213000, 'public',
             'https://i1.sndcdn.com/artworks-{SEEDED_TRACK}-t500x500.jpg',
             'https://soundcloud.com/kavinsky-fan/track-{SEEDED_TRACK}',
             'https://wave.sndcdn.com/{SEEDED_TRACK}_m.json', '{OWNER}', 'soundcloud:users:{OWNER}', 'kavinsky-fan',
             '2026-01-15 12:00:00+00', '2026-09-01 10:00:00+00', '{synced}', 517000, 5, 5, 'ok', 'indexed',
             11000, '2026-01-15 12:00:05+00', '{synced}', '{synced}');
         INSERT INTO public.users (
             sc_user_id, urn, username, username_normalized, full_name, city, country, avatar_url, verified,
             followers_count, kind, sc_last_modified, sc_synced_at, sc_observation, created_at, updated_at)
         VALUES ('{OWNER}', 'soundcloud:users:{OWNER}', 'kavinsky-fan', 'kavinsky fan', 'Night Rider', 'Paris', 'FR',
             'https://i1.sndcdn.com/avatars-{OWNER}-large.jpg', false, 812, 'user',
             '2026-08-01 09:00:00+00', '{synced}', 517001, '2025-11-02 18:00:00+00', '{synced}');
         INSERT INTO public.playlists (
             urn, sc_playlist_id, title, title_normalized, track_count, sharing, owner_sc_user_id, owner_urn,
             sc_last_modified, sc_synced_at, sc_observation, created_at, updated_at)
         VALUES ('soundcloud:playlists:1990011', '1990011', 'Drive Mix', 'drive mix', 12, 'public', '{OWNER}',
             'soundcloud:users:{OWNER}', '2026-08-10 09:00:00+00', '{synced}', 517002,
             '2026-02-01 10:00:00+00', '{synced}');
         INSERT INTO public.playlist_membership_state (
             playlist_urn, projection_track_count, sync_status, next_reconcile_at, updated_at)
         VALUES ('soundcloud:playlists:1990011', 12, 'clean', '2026-11-30 00:00:00+00', '{synced}');
         INSERT INTO public.user_profiles (soundcloud_user_id, profile_json, synced_at, sc_observation)
         VALUES ('{OWNER}', '{{\"urn\": \"soundcloud:users:{OWNER}\", \"username\": \"kavinsky-fan\"}}', '{synced}', 517001)"
    )
}

fn user(sc_id: u64, username: &str) -> Value {
    json!({
        "id": sc_id,
        "kind": "user",
        "urn": format!("soundcloud:users:{sc_id}"),
        "username": username,
        "permalink": username,
        "permalink_url": format!("https://soundcloud.com/{username}"),
        "avatar_url": format!("https://i1.sndcdn.com/avatars-{sc_id}-large.jpg"),
        "verified": false,
        "followers_count": 815,
        "track_count": 44,
        "last_modified": "2026-10-29T09:00:00Z",
    })
}

fn track(sc_id: u64, title: &str, last_modified: &str, uploader: Value) -> Value {
    json!({
        "kind": "track",
        "id": sc_id,
        "urn": format!("soundcloud:tracks:{sc_id}"),
        "title": title,
        "description": "night ride",
        "genre": "Synthwave",
        "tag_list": "synthwave nightdrive",
        "duration": 213000,
        "full_duration": 213000,
        "artwork_url": format!("https://i1.sndcdn.com/artworks-{sc_id}-t500x500.jpg"),
        "permalink_url": format!("https://soundcloud.com/kavinsky-fan/track-{sc_id}"),
        "waveform_url": format!("https://wave.sndcdn.com/{sc_id}_m.json"),
        "sharing": "public",
        "created_at": "2026-01-15T12:00:00Z",
        "last_modified": last_modified,
        "release_date": "2026-01-15T00:00:00Z",
        "publisher_metadata": {"isrc": "FRX202600031", "artist": "Kavinsky Fan"},
        "user": uploader,
        "playback_count": 12004,
        "likes_count": 431,
        "reposts_count": 17,
        "comment_count": 9,
    })
}

fn owned(sc_id: u64, title: &str, last_modified: &str) -> Value {
    track(sc_id, title, last_modified, user(OWNER, "kavinsky-fan"))
}

fn playlist(sc_id: u64, title: &str, track_count: i64, owner: Value) -> Value {
    json!({
        "kind": "playlist",
        "id": sc_id,
        "urn": format!("soundcloud:playlists:{sc_id}"),
        "title": title,
        "genre": "Synthwave",
        "tag_list": "drive",
        "permalink_url": format!("https://soundcloud.com/kavinsky-fan/sets/{sc_id}"),
        "user": owner,
        "track_count": track_count,
        "duration": 2400000,
        "set_type": "playlist",
        "sharing": "public",
        "likes_count": 31,
        "reposts_count": 2,
        "created_at": "2026-02-01T10:00:00Z",
        "last_modified": "2026-10-29T10:00:00Z",
    })
}

async fn observations(twin: &Twin, count: usize) -> Vec<Observation> {
    let mut observed = Vec::with_capacity(count);
    for _ in 0..count {
        observed.push(Observation::begin(&twin.split).await.unwrap());
    }
    observed
}

#[tokio::test]
async fn the_same_observation_stream_gives_the_same_rows_on_both_paths() {
    let mut twin = Twin::new(&seed()).await;
    let o = observations(&twin, 12).await;
    assert!(o.windows(2).all(|w| w[0].sequence() < w[1].sequence()));
    let first = owned(
        SEEDED_TRACK,
        "Night Drive (Remastered)",
        "2026-10-29T10:00:00Z",
    );
    let third = owned(SEEDED_TRACK, "Night Drive (Final)", "2026-10-29T12:00:00Z");
    let backlog = owned(SEEDED_TRACK, "Night Drive (Draft)", "2026-10-29T11:00:00Z");
    let fresh = owned(NEW_TRACK, "Sunset", "2026-10-29T10:00:00Z");
    let absent = [
        owned(SEEDED_TRACK, "Ignored", "2026-10-29T10:00:00Z"),
        fresh.clone(),
        owned(ABSENT_TRACK, "Afterglow", "2026-10-29T10:00:00Z"),
        track(
            DELETED_TRACK,
            "Gone Soon",
            "2026-10-29T10:00:00Z",
            user(OTHER_USER, "drive-club"),
        ),
    ];
    let wanted = owned(NEW_TRACK, "Wanted", "2026-10-29T10:00:00Z");
    let other = owned(NEW_TRACK, "Not Wanted", "2026-10-29T10:00:00Z");
    let mark_deleted = format!(
        "UPDATE public.tracks SET deleted_at = '2026-10-29 00:00:00+00' WHERE sc_track_id = '{DELETED_TRACK}'"
    );
    let age = format!(
        "UPDATE public.tracks SET sc_synced_at = sc_synced_at - interval '8 days' WHERE sc_track_id = '{SEEDED_TRACK}'"
    );
    let pending = format!(
        "UPDATE public.tracks SET sc_desired = '{{\"title\": \"Wanted\"}}', sc_mutation_observation = {}
         WHERE sc_track_id = '{NEW_TRACK}'",
        o[8].sequence()
    );
    let absent_id = ABSENT_TRACK.to_string();
    let steps = [
        Step::Track(&first, TrackPriority::Played, o[0]),
        Step::Track(&first, TrackPriority::Discovery, o[1]),
        Step::Track(&first, TrackPriority::Like, o[1]),
        Step::Track(&third, TrackPriority::Like, o[3]),
        Step::Track(&backlog, TrackPriority::Like, o[2]),
        Step::Track(&fresh, TrackPriority::Like, o[4]),
        Step::AbsentTracks(&absent, Observation::UNVERIFIED),
        Step::Bump(&absent_id, TrackPriority::Like),
        Step::Track(&absent[2], TrackPriority::Played, o[5]),
        Step::Sql(&mark_deleted),
        Step::Track(&absent[3], TrackPriority::Like, o[6]),
        Step::Sql(&age),
        Step::Track(&third, TrackPriority::Discovery, o[7]),
        Step::Sql(&pending),
        Step::Track(&other, TrackPriority::Discovery, o[9]),
        Step::Track(&wanted, TrackPriority::Discovery, o[10]),
        Step::Track(&backlog, TrackPriority::Like, Observation::UNVERIFIED),
    ];
    let mut outcomes = Vec::new();
    for step in steps {
        outcomes.push(twin.step(step).await);
    }
    let applied = |n: usize| outcomes[n]["applied"].as_bool().unwrap();
    assert!(applied(0) && !applied(1) && !applied(2) && applied(3));
    assert!(!applied(4), "the backlog observation is refused");
    assert_eq!(outcomes[5], json!({"was_new": true, "applied": true}));
    assert!(!applied(10), "a deleted track is not revived");
    assert!(applied(12), "a week-old sync is refreshed");
    assert!(!applied(16), "an unverified observation never overwrites");
    let title: String = sqlx::query_scalar(&format!(
        "SELECT title FROM public.tracks WHERE sc_track_id = '{SEEDED_TRACK}'"
    ))
    .fetch_one(&twin.split)
    .await
    .unwrap();
    assert_eq!(title, "Night Drive (Final)");
    let report = twin.assert_same().await;
    println!(
        "tracks stream, {} steps:\n{}",
        twin.steps,
        report.join("\n")
    );
    twin.finish().await;
}

#[tokio::test]
async fn users_playlists_and_profiles_follow_the_same_guards_on_both_paths() {
    let mut twin = Twin::new(&seed()).await;
    let o = observations(&twin, 8).await;
    let full = user(OWNER, "kavinsky-fan2");
    let thin = json!({
        "urn": format!("soundcloud:users:{OWNER}"),
        "username": "kavinsky-fan3",
        "avatar_url": null,
        "last_modified": "2026-10-29T09:00:00Z",
    });
    let older = user(OWNER, "kavinsky-old");
    let invalid = json!({"urn": format!("soundcloud:users:{OWNER}"), "username": " "});
    let absent_users = [user(OWNER, "ignored"), user(THIRD_USER, "third-one")];
    let mix = playlist(1990011, "Drive Mix", 13, user(OWNER, "kavinsky-fan"));
    let mix_backlog = playlist(1990011, "Drive Mix (old)", 11, user(OWNER, "kavinsky-fan"));
    let fresh = playlist(1990012, "Night Set", 5, user(OWNER, "kavinsky-fan"));
    let absent_playlists = [
        mix.clone(),
        fresh.clone(),
        playlist(1990013, "Morning Set", 7, user(OTHER_USER, "drive-club")),
    ];
    let untitled = json!({"urn": "soundcloud:playlists:1990014", "title": ""});
    let profile = json!({
        "urn": format!("soundcloud:users:{OWNER}"),
        "username": "kavinsky-fan4",
        "full_name": "Night Rider II",
        "plan": "Free",
        "last_modified": "2026-10-30T09:00:00Z",
    });
    let stranger = json!({"urn": "soundcloud:users:1", "username": "someone"});
    let owner = OWNER.to_string();
    let steps = [
        Step::User(&full, o[1]),
        Step::User(&thin, o[2]),
        Step::User(&older, o[0]),
        Step::User(&invalid, o[3]),
        Step::AbsentUsers(&absent_users, Observation::UNVERIFIED),
        Step::Playlist(&mix, o[3]),
        Step::Playlist(&mix, o[4]),
        Step::Playlist(&mix_backlog, o[2]),
        Step::Playlist(&fresh, o[5]),
        Step::AbsentPlaylists(&absent_playlists, Observation::UNVERIFIED),
        Step::Playlist(&untitled, o[5]),
        Step::Profile(&owner, &profile, o[6]),
        Step::Profile(&owner, &profile, o[5]),
        Step::Profile(&owner, &stranger, o[7]),
    ];
    let mut outcomes = Vec::new();
    for step in steps {
        outcomes.push(twin.step(step).await);
    }
    assert_eq!(outcomes[0], json!(false));
    assert_eq!(outcomes[3]["protocol"], json!(true));
    assert_eq!(outcomes[8], json!(true));
    assert_eq!(outcomes[13]["protocol"], json!(true));
    let report = twin.assert_same().await;
    println!(
        "users, playlists and profiles stream, {} steps:\n{}",
        twin.steps,
        report.join("\n")
    );
    twin.finish().await;
}
