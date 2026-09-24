use std::collections::HashSet;

use sqlx::PgPool;
use uuid::Uuid;

use super::clusters::ClusterResponse;
use super::live_fixture::{
    ARTISTS, LISTENER, PER_CLUSTER, catalogue, cluster_names, install_all_vectors, install_catalog,
    likes_the_same_ten, offered_ids, service,
};
use super::service::RecommendationsService;

async fn artist_of(pg: &PgPool, name: &str) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar("SELECT id FROM artists WHERE name = $1")
        .bind(name)
        .fetch_one(pg)
        .await?)
}

async fn artist_wave(
    service: &RecommendationsService,
    artist_id: Uuid,
) -> anyhow::Result<ClusterResponse> {
    Ok(Box::pin(service.artist_wave(artist_id, LISTENER, PER_CLUSTER, false)).await?)
}

async fn tracks_of(pg: &PgPool, artist_id: Uuid) -> anyhow::Result<HashSet<String>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT it.sc_track_id
         FROM track_artists ta
         JOIN tracks it ON it.id = ta.track_id
         WHERE ta.artist_id = $1 AND ta.role = 'primary'",
    )
    .bind(artist_id)
    .fetch_all(pg)
    .await?;
    Ok(ids.into_iter().collect())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn the_artists_own_tracks_are_on_the_page_and_only_on_their_own_shelf(
    pg: PgPool,
) -> anyhow::Result<()> {
    let tracks = catalogue(990_000, 60);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, 990_000).await?;

    let service = service(pg.clone()).await?;
    install_all_vectors(&service, &tracks).await?;

    let artist_id = artist_of(&pg, ARTISTS[0]).await?;
    let theirs = tracks_of(&pg, artist_id).await?;
    let response = artist_wave(&service, artist_id).await?;
    let names = cluster_names(&response);

    assert!(
        names.contains(&"essence"),
        "the artist's own best tracks are the point of their page, and the shelf is gone: \
         {names:?}"
    );
    let essence: Vec<String> = response
        .clusters
        .iter()
        .find(|cluster| cluster.id == "essence")
        .map(|cluster| cluster.track_ids.clone())
        .expect("the shelf is there");
    assert!(
        essence.iter().all(|id| theirs.contains(id)),
        "`essence` offered something the artist did not record: {essence:?}"
    );
    for cluster in &response.clusters {
        if cluster.id == "essence" {
            continue;
        }
        assert!(
            cluster.track_ids.iter().all(|id| !theirs.contains(id)),
            "`{}` repeats the artist's own tracks instead of leaving them to `essence`: {:?}",
            cluster.id,
            cluster.track_ids
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn nothing_is_offered_twice_on_an_artist_page(pg: PgPool) -> anyhow::Result<()> {
    let tracks = catalogue(991_000, 60);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, 991_000).await?;

    let service = service(pg.clone()).await?;
    install_all_vectors(&service, &tracks).await?;

    let artist_id = artist_of(&pg, ARTISTS[1]).await?;
    let response = artist_wave(&service, artist_id).await?;
    let offered = offered_ids(&response);

    assert!(
        response.clusters.len() >= 3,
        "only {} shelves came back, too few to prove anything about repeats: {:?}",
        response.clusters.len(),
        cluster_names(&response)
    );
    let mut seen = HashSet::new();
    for id in &offered {
        assert!(
            seen.insert(id.clone()),
            "{id} is on two shelves of one artist page: {offered:?}"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn an_artist_we_have_no_tracks_for_gives_an_empty_page(pg: PgPool) -> anyhow::Result<()> {
    let tracks = catalogue(992_000, 20);
    install_catalog(&pg, &tracks).await?;
    let service = service(pg).await?;
    install_all_vectors(&service, &tracks).await?;

    let response = artist_wave(&service, Uuid::now_v7()).await?;

    assert!(
        offered_ids(&response).is_empty(),
        "an artist nobody has a track for produced a page: {:?}",
        cluster_names(&response)
    );
    Ok(())
}
