use std::collections::HashSet;

use sqlx::PgPool;

use super::clusters::ClusterResponse;
use super::live_fixture::{
    LISTENER, PER_CLUSTER, by_one_artist, catalogue, cluster_names, in_cluster,
    install_all_vectors, install_catalog, likes_the_same_ten, offered_ids, service,
};
use super::service::RecommendationsService;

async fn similar_wave(
    service: &RecommendationsService,
    anchor: &str,
) -> anyhow::Result<ClusterResponse> {
    Ok(Box::pin(service.similar_wave(anchor, LISTENER, None, PER_CLUSTER, false)).await?)
}

fn assert_the_shelves_were_built(response: &ClusterResponse) {
    let names = cluster_names(response);
    for expected in ["wave", "same_artist", "same_vibe"] {
        assert!(
            names.contains(&expected),
            "`{expected}` is missing, so this answer never reached Qdrant or the catalogue \
             and the test would pass over an empty page: {names:?}"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn the_track_you_are_listening_to_is_never_offered_next_to_itself(
    pg: PgPool,
) -> anyhow::Result<()> {
    let tracks = catalogue(940_000, 60);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, 940_000).await?;

    let service = service(pg).await?;
    install_all_vectors(&service, &tracks).await?;

    let anchor = "940020";
    let response = similar_wave(&service, anchor).await?;

    assert_the_shelves_were_built(&response);
    assert!(
        !offered_ids(&response).contains(&anchor.to_owned()),
        "the anchor came back in its own list of neighbours: {:?}",
        offered_ids(&response)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn nothing_is_offered_twice_across_the_shelves(pg: PgPool) -> anyhow::Result<()> {
    let tracks = catalogue(950_000, 60);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, 950_000).await?;

    let service = service(pg).await?;
    install_all_vectors(&service, &tracks).await?;

    let response = similar_wave(&service, "950030").await?;
    let offered = offered_ids(&response);

    assert_the_shelves_were_built(&response);
    let mut seen = HashSet::new();
    for id in &offered {
        assert!(
            seen.insert(id.clone()),
            "{id} is on two shelves of the same page: {offered:?}"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn the_same_vibe_shelf_does_not_repeat_the_anchors_own_artist(
    pg: PgPool,
) -> anyhow::Result<()> {
    let tracks = by_one_artist(960_000, 40);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, 960_000).await?;

    let service = service(pg).await?;
    install_all_vectors(&service, &tracks).await?;

    let response = similar_wave(&service, "960020").await?;
    let names = cluster_names(&response);

    assert!(
        names.contains(&"same_artist"),
        "the anchor's artist has thirty nine other tracks, so this shelf must exist: {names:?}"
    );
    let same_vibe: Vec<String> = response
        .clusters
        .iter()
        .find(|cluster| cluster.id == "same_vibe")
        .map(|cluster| cluster.track_ids.clone())
        .unwrap_or_default();
    assert!(
        same_vibe.is_empty(),
        "every track in this catalogue belongs to the anchor's own artist, so `same_vibe` \
         has nothing honest to offer and must stay empty: {same_vibe:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_neighbour_by_another_artist_still_reaches_the_same_vibe_shelf(
    pg: PgPool,
) -> anyhow::Result<()> {
    let tracks = catalogue(970_000, 60);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, 970_000).await?;

    let service = service(pg).await?;
    install_all_vectors(&service, &tracks).await?;

    let response = similar_wave(&service, "970020").await?;
    let same_vibe: Vec<String> = response
        .clusters
        .iter()
        .find(|cluster| cluster.id == "same_vibe")
        .map(|cluster| cluster.track_ids.clone())
        .unwrap_or_default();

    assert!(
        !same_vibe.is_empty(),
        "with four artists in the catalogue the shelf must fill, otherwise the guard above \
         proves nothing: {:?}",
        cluster_names(&response)
    );
    for id in &same_vibe {
        assert!(
            in_cluster(&response, id) == vec!["same_vibe"],
            "{id} is on `same_vibe` and on another shelf at once"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_track_we_have_never_heard_of_gives_an_empty_page_instead_of_an_error(
    pg: PgPool,
) -> anyhow::Result<()> {
    let tracks = catalogue(980_000, 20);
    install_catalog(&pg, &tracks).await?;
    let service = service(pg).await?;
    install_all_vectors(&service, &tracks).await?;

    for anchor in ["not-a-number", "", "0", "18446744073709551616"] {
        let response = similar_wave(&service, anchor).await?;
        assert!(
            offered_ids(&response).is_empty(),
            "`{anchor}` is not a track of ours, yet it produced a page: {:?}",
            cluster_names(&response)
        );
    }
    Ok(())
}
