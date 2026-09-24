#[cfg(test)]
#[path = "vectors_live_tests.rs"]
mod live_tests;

use futures::future::BoxFuture;

use crate::qdrant::QdrantProvisioner;

pub trait LyricsVectorStore: Send + Sync {
    fn upsert_lyrics<'a>(
        &'a self,
        sc_track_id: u64,
        embedding_request_id: &'a str,
        language: Option<&'a str>,
        vector: &'a [f32],
    ) -> BoxFuture<'a, anyhow::Result<()>>;
}

impl LyricsVectorStore for QdrantProvisioner {
    fn upsert_lyrics<'a>(
        &'a self,
        sc_track_id: u64,
        embedding_request_id: &'a str,
        language: Option<&'a str>,
        vector: &'a [f32],
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(QdrantProvisioner::upsert_lyrics(
            self,
            sc_track_id,
            embedding_request_id,
            language,
            vector.to_vec(),
        ))
    }
}
