use std::sync::Arc;

use backend_contracts::vector_store::{TRACKS_LYRICS, TRACKS_LYRICS_DIMENSIONS};
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{DeletePointsBuilder, GetPointsBuilder, PointId, PointsIdsList};

use crate::config::QdrantConfig;

use super::*;

const POINT_ID: u64 = 991_100_042;

fn grpc_url() -> String {
    std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6334".to_owned())
}

#[tokio::test]
#[ignore = "requires a local Qdrant instance"]
async fn the_jobs_vector_store_writes_lyrics_points_with_their_request() -> anyhow::Result<()> {
    let qdrant = QdrantProvisioner::connect(&QdrantConfig {
        grpc_url: grpc_url(),
        api_key: String::new().into(),
    })?;
    qdrant.provision().await?;
    let store: Arc<dyn LyricsVectorStore> = Arc::new(qdrant);
    let vector = vec![0.25; usize::try_from(TRACKS_LYRICS_DIMENSIONS)?];

    store
        .upsert_lyrics(POINT_ID, "lyr:live:1", Some("en"), &vector)
        .await?;

    let client = Qdrant::from_url(&grpc_url())
        .skip_compatibility_check()
        .build()?;
    let response = client
        .get_points(
            GetPointsBuilder::new(TRACKS_LYRICS, vec![PointId::from(POINT_ID)]).with_payload(true),
        )
        .await?;
    client
        .delete_points(
            DeletePointsBuilder::new(TRACKS_LYRICS)
                .points(PointsIdsList {
                    ids: vec![PointId::from(POINT_ID)],
                })
                .wait(true),
        )
        .await?;
    let payload = response
        .result
        .into_iter()
        .next()
        .map(|point| point.payload)
        .ok_or_else(|| anyhow::anyhow!("lyrics point was not stored"))?;
    let text = |key: &str| {
        payload
            .get(key)
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned)
    };
    assert_eq!(text("embedding_request_id").as_deref(), Some("lyr:live:1"));
    assert_eq!(text("language").as_deref(), Some("en"));
    assert_eq!(text("sc_track_id").as_deref(), Some("991100042"));
    Ok(())
}
