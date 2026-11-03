use serde_json::{Value, json};
use sqlx::PgPool;

use super::twin::{Step, Twin};
use crate::{Observation, TrackPriority};

const LOADER: &str = "CREATE SCHEMA sample;
CREATE FUNCTION sample.load(tbl text, rows jsonb) RETURNS void LANGUAGE plpgsql AS $f$
DECLARE
    cols text;
BEGIN
    SELECT string_agg(quote_ident(a.attname), ', ' ORDER BY a.attnum) INTO cols
    FROM pg_attribute a
    WHERE a.attrelid = ('public.' || tbl)::regclass AND a.attnum > 0 AND NOT a.attisdropped
      AND a.attgenerated = '' AND rows -> 0 ? a.attname;
    EXECUTE format('INSERT INTO public.%I (%s) SELECT %s FROM jsonb_populate_recordset(NULL::public.%I, $1)', tbl, cols, cols, tbl)
    USING rows;
END
$f$;";

const TRACK_PAYLOADS: &str = "SELECT jsonb_build_object(
        'kind', 'track', 'urn', urn, 'title', title, 'description', description, 'genre', genre,
        'tag_list', array_to_string(tags, ' '), 'full_duration', duration_ms, 'artwork_url', artwork_url,
        'permalink_url', permalink_url, 'waveform_url', waveform_url, 'language', language, 'isrc', isrc,
        'metadata_artist', metadata_artist, 'sharing', sharing, 'created_at', sc_created_at,
        'last_modified', sc_last_modified, 'release_date', release_date,
        'user', jsonb_build_object('urn', uploader_urn, 'username', uploader_username, 'avatar_url', uploader_avatar_url),
        'playback_count', play_count_sc, 'likes_count', likes_count_sc, 'reposts_count', reposts_count_sc,
        'comment_count', comments_count_sc)
    FROM public.tracks ORDER BY sc_track_id";

const USER_PAYLOADS: &str = "SELECT jsonb_build_object(
        'kind', kind, 'urn', urn, 'username', username, 'full_name', full_name, 'first_name', first_name,
        'last_name', last_name, 'permalink', permalink, 'permalink_url', permalink_url, 'avatar_url', avatar_url,
        'country_code', country, 'city', city, 'description', description, 'verified', verified,
        'followers_count', followers_count, 'followings_count', followings_count, 'track_count', tracks_count,
        'playlist_count', playlists_count, 'reposts_count', reposts_count, 'comments_count', comments_count,
        'created_at', sc_created_at, 'last_modified', sc_last_modified)
    FROM public.users ORDER BY sc_user_id";

const PLAYLIST_PAYLOADS: &str = "SELECT jsonb_build_object(
        'kind', kind, 'urn', urn, 'title', title, 'description', description, 'genre', genre,
        'tag_list', array_to_string(tags, ' '), 'artwork_url', artwork_url, 'permalink_url', permalink_url,
        'user', jsonb_build_object('urn', owner_urn, 'username', owner_username), 'track_count', track_count,
        'duration', duration_ms, 'set_type', playlist_type, 'sharing', sharing, 'release_date', release_date,
        'label_name', label_name, 'likes_count', likes_count_sc, 'reposts_count', reposts_count_sc,
        'created_at', sc_created_at, 'last_modified', sc_last_modified)
    FROM public.playlists ORDER BY urn";

async fn payloads(db: &PgPool, sql: &str) -> Vec<Value> {
    sqlx::query_scalar(sql).fetch_all(db).await.unwrap()
}

fn with(payload: &Value, key: &str, value: Value) -> Value {
    let mut changed = payload.clone();
    changed[key] = value;
    changed
}

#[tokio::test]
#[ignore = "needs REPLAY_SAMPLE, the path of a small read-only prod sample"]
async fn a_prod_sample_replays_to_the_same_rows_on_both_paths() {
    let path = std::env::var("REPLAY_SAMPLE").expect("REPLAY_SAMPLE");
    let sample = std::fs::read_to_string(path).unwrap();
    let seed = format!(
        "{LOADER}
         SELECT sample.load(t.key, t.value) FROM jsonb_each($sample${sample}$sample$::jsonb) AS t
         WHERE jsonb_array_length(t.value) > 0"
    );
    let mut twin = Twin::new(&seed).await;
    let tracks = payloads(&twin.split, TRACK_PAYLOADS).await;
    let users = payloads(&twin.split, USER_PAYLOADS).await;
    let playlists = payloads(&twin.split, PLAYLIST_PAYLOADS).await;
    let profiles: Vec<(String, Value)> = sqlx::query_as(
        "SELECT soundcloud_user_id, profile_json FROM public.user_profiles ORDER BY 1",
    )
    .fetch_all(&twin.split)
    .await
    .unwrap();
    let mut o = Vec::new();
    for _ in 0..4 {
        o.push(Observation::begin(&twin.split).await.unwrap());
    }
    let mut applied = [0; 3];
    for (round, (priority, observation)) in [
        (TrackPriority::Played, o[0]),
        (TrackPriority::Like, o[2]),
        (TrackPriority::Like, o[1]),
    ]
    .into_iter()
    .enumerate()
    {
        for track in &tracks {
            let title = track["title"].as_str().unwrap_or_default();
            let payload = match round {
                1 => with(track, "title", json!(format!("{title} (edit)"))),
                _ => track.clone(),
            };
            let outcome = twin
                .step(Step::Track(&payload, priority, observation))
                .await;
            applied[round] += usize::from(outcome["applied"] == json!(true));
        }
    }
    let mut absent = tracks.clone();
    absent.push(with(
        &tracks[0],
        "urn",
        json!("soundcloud:tracks:9000000000001"),
    ));
    twin.step(Step::AbsentTracks(&absent, Observation::UNVERIFIED))
        .await;
    for (round, observation) in [o[0], o[2], o[1]].into_iter().enumerate() {
        for user in &users {
            let payload = match round {
                1 => json!({"urn": user["urn"], "username": user["username"], "avatar_url": null}),
                _ => user.clone(),
            };
            twin.step(Step::User(&payload, observation)).await;
        }
        for playlist in &playlists {
            let count = playlist["track_count"].as_i64().unwrap_or(0);
            let payload = match round {
                1 => with(playlist, "track_count", json!(count + 1)),
                _ => playlist.clone(),
            };
            twin.step(Step::Playlist(&payload, observation)).await;
        }
    }
    twin.step(Step::AbsentPlaylists(&playlists, Observation::UNVERIFIED))
        .await;
    for (user_id, profile) in &profiles {
        twin.step(Step::Profile(user_id, profile, o[3])).await;
    }
    let report = twin.assert_same().await;
    println!(
        "prod sample: {} tracks, {} users, {} playlists, {} profiles, {} steps\n\
         tracks applied per round (same payload, edited, backlog): {applied:?}\n{}",
        tracks.len(),
        users.len(),
        playlists.len(),
        profiles.len(),
        twin.steps,
        report.join("\n")
    );
    twin.finish().await;
}
