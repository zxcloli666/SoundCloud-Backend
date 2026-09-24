use std::collections::HashSet;

use sqlx::PgPool;

use super::clusters::ClusterResponse;
use super::home_wave::HomeRequest;
use super::live_fixture::{
    LIKES, LISTENER, PER_CLUSTER, TWIN, assert_the_vector_clusters_were_built, by_one_artist,
    catalogue, cluster_names, dislike, install_all_vectors, install_catalog, like,
    likes_the_same_ten, offered_by_vectors, offered_ids, service,
};
use super::service::RecommendationsService;

const TASTE_CLUSTERS: [&str; 2] = ["same_vibe", "deep_cuts"];

async fn home_wave(
    service: &RecommendationsService,
    listener: &str,
) -> anyhow::Result<ClusterResponse> {
    Ok(Box::pin(service.home_wave(HomeRequest {
        sc_user_id: listener.to_owned(),
        languages: None,
        per_cluster: PER_CLUSTER,
        hide_listened: false,
    }))
    .await?)
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn two_listeners_with_the_same_taste_are_offered_the_same_wave(
    pg: PgPool,
) -> anyhow::Result<()> {
    let tracks = catalogue(870_000, 60);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, 870_000).await?;
    likes_the_same_ten(&pg, TWIN, 870_000).await?;

    let service = service(pg).await?;
    install_all_vectors(&service, &tracks).await?;

    let mine = home_wave(&service, LISTENER).await?;
    let theirs = home_wave(&service, TWIN).await?;

    assert_the_vector_clusters_were_built(&mine);
    assert_eq!(
        offered_ids(&mine).into_iter().collect::<HashSet<_>>(),
        offered_ids(&theirs).into_iter().collect::<HashSet<_>>(),
        "the same signals over the same catalogue must give the same wave, otherwise every \
         comparison between two listeners in this file proves nothing"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_user_without_a_single_signal_is_shown_the_cold_start_shelf(
    pg: PgPool,
) -> anyhow::Result<()> {
    let tracks = catalogue(880_000, 10);
    install_catalog(&pg, &tracks).await?;
    let service = service(pg).await?;
    install_all_vectors(&service, &tracks).await?;

    let response = home_wave(&service, LISTENER).await?;

    assert_eq!(
        cluster_names(&response),
        vec!["discover"],
        "a user we know nothing about gets the cold start shelf and only that"
    );
    let offered = offered_ids(&response);
    assert_eq!(
        offered.len(),
        PER_CLUSTER,
        "the shelf is filled to what the caller asked for, not left half empty"
    );
    let known: HashSet<String> = tracks
        .iter()
        .map(|track| track.sc_track_id.to_string())
        .collect();
    assert!(
        offered.iter().all(|id| known.contains(id)),
        "the shelf offered a track that is not in the catalogue: {offered:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_taste_too_thin_for_vectors_still_gets_something_to_discover(
    pg: PgPool,
) -> anyhow::Result<()> {
    let tracks = catalogue(940_000, 40);
    install_catalog(&pg, &tracks).await?;
    for index in 0..3 {
        like(&pg, LISTENER, 940_000 + index).await?;
    }
    likes_the_same_ten(&pg, TWIN, 940_000).await?;

    let service = service(pg).await?;
    install_all_vectors(&service, &tracks).await?;

    let thin = home_wave(&service, LISTENER).await?;
    let known = home_wave(&service, TWIN).await?;
    let thin_names = cluster_names(&thin);
    let known_names = cluster_names(&known);
    assert_the_vector_clusters_were_built(&known);
    assert!(
        thin_names.contains(&"wave"),
        "the wave itself seeds straight from the likes and must survive a thin taste: \
         {thin_names:?}"
    );

    for shelf in TASTE_CLUSTERS {
        assert!(
            known_names.contains(&shelf),
            "`{shelf}` is missing even for a listener whose taste is known, so its absence \
             below the threshold would prove nothing: {known_names:?}"
        );
        assert!(
            !thin_names.contains(&shelf),
            "`{shelf}` is built from the taste centroid, and three likes are below the \
             threshold that fills it: {thin_names:?}"
        );
    }
    assert!(
        thin_names.contains(&"discover"),
        "without the taste shelves this listener is left with a page half the size, and \
         nothing offers him anything he has not already found: {thin_names:?}"
    );
    assert!(
        !known_names.contains(&"discover"),
        "a listener whose taste is known must be answered from it, not from what is merely \
         popular: {known_names:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_disliked_track_is_absent_from_every_cluster(pg: PgPool) -> anyhow::Result<()> {
    let tracks = catalogue(890_000, 60);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, 890_000).await?;
    likes_the_same_ten(&pg, TWIN, 890_000).await?;

    let service = service(pg.clone()).await?;
    install_all_vectors(&service, &tracks).await?;

    let theirs = home_wave(&service, TWIN).await?;
    assert_the_vector_clusters_were_built(&theirs);
    let liked: HashSet<String> = (0..LIKES)
        .map(|index| (890_000 + index).to_string())
        .collect();
    let doomed_in_each: Vec<(&str, String)> = theirs
        .clusters
        .iter()
        .filter_map(|cluster| {
            cluster
                .track_ids
                .iter()
                .find(|id| !liked.contains(*id))
                .map(|id| (cluster.id, id.clone()))
        })
        .collect();
    assert!(
        doomed_in_each.len() >= 4,
        "only {} clusters offered anything new, so this test would only cover part of the wave",
        doomed_in_each.len()
    );

    for (_, doomed) in &doomed_in_each {
        dislike(&pg, LISTENER, doomed).await?;
    }
    let mine = home_wave(&service, LISTENER).await?;

    assert_the_vector_clusters_were_built(&mine);
    for (cluster, doomed) in &doomed_in_each {
        assert!(
            !offered_ids(&mine).contains(doomed),
            "`{cluster}` offered {doomed} to the listener next door and offers it here too, \
             to someone who disliked it"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn no_track_is_offered_twice_in_one_wave(pg: PgPool) -> anyhow::Result<()> {
    let tracks = catalogue(900_000, 60);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, 900_000).await?;

    let service = service(pg).await?;
    install_all_vectors(&service, &tracks).await?;

    let response = home_wave(&service, LISTENER).await?;
    let offered = offered_ids(&response);

    assert_the_vector_clusters_were_built(&response);
    let mut seen = HashSet::new();
    for id in &offered {
        assert!(
            seen.insert(id.clone()),
            "{id} was offered twice in one wave: {offered:?}"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_track_that_is_too_short_is_never_recommended(pg: PgPool) -> anyhow::Result<()> {
    let tracks = catalogue(910_000, 10);
    install_catalog(&pg, &tracks).await?;
    let service = service(pg.clone()).await?;
    install_all_vectors(&service, &tracks).await?;

    let before = home_wave(&service, TWIN).await?;
    let stub = offered_ids(&before)
        .into_iter()
        .next()
        .expect("the cold start shelf offers something");
    sqlx::query("UPDATE tracks SET duration_ms = 9000 WHERE sc_track_id = $1")
        .bind(&stub)
        .execute(&pg)
        .await?;

    let after = home_wave(&service, LISTENER).await?;
    let offered = offered_ids(&after);

    assert!(
        !offered.is_empty(),
        "the shelf came back empty, so this test would pass for the wrong reason"
    );
    assert!(
        !offered.contains(&stub),
        "{stub} was offered before it was cut to nine seconds and is still offered: {offered:?}"
    );
    Ok(())
}

const GHOSTS: [(&str, &str); 4] = [
    ("private", "sharing = 'private'"),
    ("not yet indexed", "index_state = 'pending'"),
    ("too long for us", "storage_state = 'too_long'"),
    ("of unknown length", "needs_duration_resolve = true"),
];

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_track_postgres_says_is_ineligible_never_comes_back_from_qdrant(
    pg: PgPool,
) -> anyhow::Result<()> {
    let tracks = catalogue(920_000, 60);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, 920_000).await?;
    likes_the_same_ten(&pg, TWIN, 920_000).await?;

    let service = service(pg.clone()).await?;
    install_all_vectors(&service, &tracks).await?;

    let theirs = home_wave(&service, TWIN).await?;
    assert_the_vector_clusters_were_built(&theirs);
    let eligible = offered_by_vectors(&theirs);
    assert!(
        eligible.len() >= GHOSTS.len(),
        "only {} tracks came from Qdrant, not enough to make one ghost of each kind",
        eligible.len()
    );

    for ((_, predicate), ghost) in GHOSTS.iter().zip(&eligible) {
        sqlx::query(&format!(
            "UPDATE tracks SET {predicate} WHERE sc_track_id = $1"
        ))
        .bind(ghost)
        .execute(&pg)
        .await?;
    }

    let mine = offered_by_vectors(&home_wave(&service, LISTENER).await?);
    for ((reason, _), ghost) in GHOSTS.iter().zip(&eligible) {
        assert!(
            !mine.contains(ghost),
            "{ghost} is {reason}, its vector is still in Qdrant, and the vector path offered \
             it anyway — it was offered to the listener next door for whom it is still fine"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn no_shelf_offers_a_track_whose_length_we_do_not_trust(pg: PgPool) -> anyhow::Result<()> {
    let tracks = by_one_artist(930_000, 11);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, 930_000).await?;
    let ghost = 930_010.to_string();
    sqlx::query("UPDATE sc_track_counters SET play_count = 999999 WHERE sc_track_id = $1")
        .bind(&ghost)
        .execute(&pg)
        .await?;

    let service = service(pg.clone()).await?;
    install_all_vectors(&service, &tracks).await?;

    let trusted = offered_ids(&home_wave(&service, LISTENER).await?);
    assert!(
        trusted.contains(&ghost),
        "the fixture must start with the track reachable, or hiding it proves nothing: \
         {trusted:?}"
    );

    sqlx::query(
        "UPDATE tracks SET needs_duration_resolve = true, duration_ms = 30000
         WHERE sc_track_id = $1",
    )
    .bind(&ghost)
    .execute(&pg)
    .await?;

    let response = home_wave(&service, LISTENER).await?;
    let holding: Vec<&str> = response
        .clusters
        .iter()
        .filter(|cluster| cluster.track_ids.contains(&ghost))
        .map(|cluster| cluster.id)
        .collect();

    assert!(
        holding.is_empty(),
        "SoundCloud gave us a preview length of 30 seconds instead of the real one, so this \
         track would be shown with a duration that is a lie. The vector path already refuses \
         it; the artist shelves must refuse it too, or the same page carries both honest and \
         invented lengths: {holding:?}"
    );
    Ok(())
}
