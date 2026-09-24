use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, bail, ensure};
use backend_contracts::vector_store::{
    CollectionProfile, CollectionSpec, REQUIRED_COLLECTIONS, TRACKS_CLAP, TRACKS_CLAP_DIMENSIONS,
    TRACKS_COLLAB, TRACKS_LYRICS, TRACKS_LYRICS_DIMENSIONS, TRACKS_MERT, TRACKS_MERT_DIMENSIONS,
    TRACKS_TASTE_DIMENSIONS,
};
use qdrant_client::config::QdrantConfig;
use qdrant_client::qdrant::{
    CollectionInfo, CollectionStatus, Condition, CountPointsBuilder, CreateAliasBuilder,
    CreateCollection, CreateCollectionBuilder, Datatype, DeletePointsBuilder, Distance, Filter,
    GetPointsBuilder, HnswConfigDiffBuilder, PointId, PointStruct, PointsOperationResponse,
    RetrievedPoint, ScrollPointsBuilder, UpdateStatus, UpsertPointsBuilder, VectorParams,
    VectorParamsBuilder, point_id::PointIdOptions, vector_output::Vector as VectorVariant,
    vectors_config::Config as VectorsConfig, vectors_output::VectorsOptions,
};
use qdrant_client::{Payload, Qdrant};

use crate::config::QdrantConfig as JobsQdrantConfig;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
const COLLAB_MODEL_KEY: &str = "model";
const VERSIONED_UPSERT_CHUNK: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AliasedCollection {
    pub alias: &'static str,
    pub prefix: &'static str,
    pub dimensions: u64,
}

pub const TRACKS_TASTE: AliasedCollection = AliasedCollection {
    alias: "tracks_taste",
    prefix: "tracks_taste_",
    dimensions: TRACKS_TASTE_DIMENSIONS,
};

pub const ALIASED_COLLECTIONS: [AliasedCollection; 1] = [TRACKS_TASTE];

pub type ScrolledVectors = (Vec<(u64, Vec<f32>)>, Option<u64>);

#[derive(Clone)]
pub struct QdrantProvisioner {
    client: Qdrant,
}

impl QdrantProvisioner {
    pub fn connect(config: &JobsQdrantConfig) -> anyhow::Result<Self> {
        let mut client_config = QdrantConfig::from_url(&config.grpc_url)
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(5))
            .skip_compatibility_check();
        if !config.api_key.is_empty() {
            client_config = client_config.api_key(config.api_key.expose().clone());
        }
        let client = Qdrant::new(client_config).context("Qdrant client initialization failed")?;
        Ok(Self { client })
    }

    pub async fn provision(&self) -> anyhow::Result<()> {
        for spec in REQUIRED_COLLECTIONS {
            provision_collection(&self.client, spec).await?;
        }
        for aliased in ALIASED_COLLECTIONS {
            validate_alias_target(&self.client, aliased).await?;
        }
        Ok(())
    }

    pub async fn is_available(&self) -> bool {
        let validation = async {
            for spec in REQUIRED_COLLECTIONS {
                validate_loaded_collection(&self.client, spec).await?;
            }
            Ok::<_, anyhow::Error>(())
        };
        matches!(
            tokio::time::timeout(HEALTH_TIMEOUT, validation).await,
            Ok(Ok(()))
        )
    }

    pub async fn retrieve_vectors(
        &self,
        collection: &str,
        ids: &[u64],
    ) -> anyhow::Result<HashMap<String, Vec<f32>>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let points = ids.into_iter().map(PointId::from).collect::<Vec<_>>();
        let response = self
            .client
            .get_points(GetPointsBuilder::new(collection, points).with_vectors(true))
            .await
            .with_context(|| format!("Qdrant collection {collection} lookup failed"))?;

        let mut vectors = HashMap::with_capacity(response.result.len());
        for point in response.result {
            let Some(id) = point.id.and_then(|id| id.point_id_options) else {
                continue;
            };
            let id = match id {
                PointIdOptions::Num(id) => id.to_string(),
                PointIdOptions::Uuid(id) => id,
            };
            let Some(VectorsOptions::Vector(vector)) =
                point.vectors.and_then(|vectors| vectors.vectors_options)
            else {
                continue;
            };
            let VectorVariant::Dense(vector) = vector.into_vector() else {
                continue;
            };
            if vector.data.iter().all(|value| value.is_finite()) {
                vectors.insert(id, vector.data);
            }
        }
        Ok(vectors)
    }

    pub async fn collab_dimension(&self) -> anyhow::Result<Option<u64>> {
        if !self
            .client
            .collection_exists(TRACKS_COLLAB)
            .await
            .context("Qdrant collab collection existence check failed")?
        {
            return Ok(None);
        }

        let info = load_collection_info(&self.client, TRACKS_COLLAB).await?;
        let dimensions = unnamed_vectors(TRACKS_COLLAB, &info)?.size;
        validate_collection(collab_spec(dimensions), &info)?;
        Ok(Some(dimensions))
    }

    pub async fn collab_points_count(&self) -> anyhow::Result<u64> {
        if !self
            .client
            .collection_exists(TRACKS_COLLAB)
            .await
            .context("Qdrant collab collection existence check failed")?
        {
            return Ok(0);
        }

        let response = self
            .client
            .count(CountPointsBuilder::new(TRACKS_COLLAB).exact(true))
            .await
            .context("Qdrant collab points could not be counted")?;
        let counted = response
            .result
            .context("Qdrant collab count returned no result")?;
        Ok(counted.count)
    }

    pub async fn upsert_collab(
        &self,
        dimensions: u64,
        model: &str,
        points: Vec<(u64, Vec<f32>)>,
    ) -> anyhow::Result<usize> {
        upsert_collab_points(&self.client, collab_spec(dimensions), model, points).await
    }

    pub async fn remove_other_collab_models(&self, model: &str) -> anyhow::Result<()> {
        remove_points_of_other_models(&self.client, TRACKS_COLLAB, model).await
    }

    pub async fn upsert_query_vector(
        &self,
        collection: &str,
        hash: &str,
        vector: Vec<f32>,
    ) -> anyhow::Result<()> {
        ensure!(
            vector.iter().all(|value| value.is_finite()),
            "query vector contains a non-finite value"
        );
        let payload = Payload::from(serde_json::Map::from_iter([(
            "created_at".to_owned(),
            serde_json::json!(chrono::Utc::now().timestamp()),
        )]));
        let point = PointStruct::new(
            backend_contracts::vector_store::query_point_uuid(hash),
            vector,
            payload,
        );
        let response = self
            .client
            .upsert_points(UpsertPointsBuilder::new(collection, vec![point]).wait(true))
            .await
            .with_context(|| {
                format!("Qdrant query vector could not be upserted into {collection}")
            })?;
        ensure_completed(response, "query vector upsert")
    }

    pub async fn upsert_audio(
        &self,
        sc_track_id: u64,
        upload_generation: i64,
        mert: Vec<f32>,
        clap: Vec<f32>,
        language: Option<&str>,
    ) -> anyhow::Result<()> {
        validate_vector(&mert, TRACKS_MERT_DIMENSIONS, "MERT")?;
        validate_vector(&clap, TRACKS_CLAP_DIMENSIONS, "CLAP")?;
        let mut payload = serde_json::Map::from_iter([
            (
                "sc_track_id".to_owned(),
                serde_json::json!(sc_track_id.to_string()),
            ),
            (
                "upload_generation".to_owned(),
                serde_json::json!(upload_generation),
            ),
            (
                "indexed_at".to_owned(),
                serde_json::json!(chrono::Utc::now().timestamp()),
            ),
        ]);
        if let Some(language) = language.filter(|language| !language.is_empty()) {
            payload.insert("language".to_owned(), serde_json::json!(language));
        }
        let payload = Payload::from(payload);

        let mert_response = self
            .client
            .upsert_points(
                UpsertPointsBuilder::new(
                    TRACKS_MERT,
                    vec![PointStruct::new(sc_track_id, mert, payload.clone())],
                )
                .wait(true),
            )
            .await
            .context("Qdrant MERT point could not be upserted")?;
        ensure_completed(mert_response, "MERT upsert")?;
        let clap_response = self
            .client
            .upsert_points(
                UpsertPointsBuilder::new(
                    TRACKS_CLAP,
                    vec![PointStruct::new(sc_track_id, clap, payload)],
                )
                .wait(true),
            )
            .await
            .context("Qdrant CLAP point could not be upserted")?;
        ensure_completed(clap_response, "CLAP upsert")?;
        Ok(())
    }

    pub async fn upsert_lyrics(
        &self,
        sc_track_id: u64,
        embedding_request_id: &str,
        language: Option<&str>,
        vector: Vec<f32>,
    ) -> anyhow::Result<()> {
        validate_vector(&vector, TRACKS_LYRICS_DIMENSIONS, "lyrics")?;
        let mut payload = serde_json::Map::from_iter([
            (
                "sc_track_id".to_owned(),
                serde_json::json!(sc_track_id.to_string()),
            ),
            (
                "embedding_request_id".to_owned(),
                serde_json::json!(embedding_request_id),
            ),
            (
                "embedded_at".to_owned(),
                serde_json::json!(chrono::Utc::now().timestamp()),
            ),
        ]);
        if let Some(language) = language.filter(|language| !language.is_empty()) {
            payload.insert("language".to_owned(), serde_json::json!(language));
        }
        let response = self
            .client
            .upsert_points(
                UpsertPointsBuilder::new(
                    TRACKS_LYRICS,
                    vec![PointStruct::new(
                        sc_track_id,
                        vector,
                        Payload::from(payload),
                    )],
                )
                .wait(true),
            )
            .await
            .context("Qdrant lyrics point could not be upserted")?;
        ensure_completed(response, "lyrics upsert")
    }

    pub async fn points_count(&self, collection: &str) -> anyhow::Result<u64> {
        let response = self
            .client
            .count(CountPointsBuilder::new(collection).exact(false))
            .await
            .with_context(|| format!("Qdrant {collection} points could not be counted"))?;
        Ok(response
            .result
            .with_context(|| format!("Qdrant {collection} count returned no result"))?
            .count)
    }

    pub async fn scroll_vectors(
        &self,
        collection: &str,
        after: Option<u64>,
        limit: u32,
    ) -> anyhow::Result<ScrolledVectors> {
        let mut request = ScrollPointsBuilder::new(collection)
            .limit(limit)
            .with_payload(false)
            .with_vectors(true);
        if let Some(after) = after {
            request = request.offset(after);
        }
        let response = self
            .client
            .scroll(request)
            .await
            .with_context(|| format!("Qdrant {collection} could not be scrolled"))?;
        let next = response
            .next_page_offset
            .and_then(|id| id.point_id_options)
            .and_then(|id| match id {
                PointIdOptions::Num(id) => Some(id),
                PointIdOptions::Uuid(_) => None,
            });
        let points = response
            .result
            .into_iter()
            .filter_map(numbered_vector)
            .collect();
        Ok((points, next))
    }

    pub async fn ensure_versioned_collection(
        &self,
        aliased: AliasedCollection,
        name: &str,
    ) -> anyhow::Result<()> {
        ensure_versioned_collection(&self.client, aliased, name).await
    }

    pub async fn upsert_versioned_points(
        &self,
        aliased: AliasedCollection,
        name: &str,
        points: Vec<(u64, Vec<f32>)>,
    ) -> anyhow::Result<usize> {
        upsert_versioned_points(&self.client, aliased, name, points).await
    }

    pub async fn alias_target(&self, aliased: AliasedCollection) -> anyhow::Result<Option<String>> {
        alias_target(&self.client, aliased).await
    }

    pub async fn point_alias(&self, aliased: AliasedCollection, name: &str) -> anyhow::Result<()> {
        point_alias(&self.client, aliased, name).await
    }

    pub async fn drop_versioned_collection(
        &self,
        aliased: AliasedCollection,
        name: &str,
    ) -> anyhow::Result<()> {
        drop_versioned_collection(&self.client, aliased, name).await
    }
}

fn numbered_vector(point: RetrievedPoint) -> Option<(u64, Vec<f32>)> {
    let PointIdOptions::Num(id) = point.id?.point_id_options? else {
        return None;
    };
    let VectorsOptions::Vector(vector) = point.vectors?.vectors_options? else {
        return None;
    };
    let VectorVariant::Dense(vector) = vector.into_vector() else {
        return None;
    };
    vector
        .data
        .iter()
        .all(|value| value.is_finite())
        .then_some((id, vector.data))
}

async fn alias_target(
    client: &Qdrant,
    aliased: AliasedCollection,
) -> anyhow::Result<Option<String>> {
    let aliases = client
        .list_aliases()
        .await
        .context("Qdrant aliases could not be listed")?;
    Ok(aliases
        .aliases
        .into_iter()
        .find(|alias| alias.alias_name == aliased.alias)
        .map(|alias| alias.collection_name))
}

async fn validate_alias_target(client: &Qdrant, aliased: AliasedCollection) -> anyhow::Result<()> {
    let Some(target) = alias_target(client, aliased).await? else {
        return Ok(());
    };
    ensure!(
        is_versioned_name(aliased, &target),
        "Qdrant alias {} points at {target}, which is not one of its versioned collections",
        aliased.alias
    );
    validate_versioned_collection(
        aliased,
        &target,
        &load_collection_info(client, &target).await?,
    )
}

fn is_versioned_name(aliased: AliasedCollection, name: &str) -> bool {
    name.strip_prefix(aliased.prefix).is_some_and(|version| {
        !version.is_empty()
            && version
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    })
}

async fn ensure_versioned_collection(
    client: &Qdrant,
    aliased: AliasedCollection,
    name: &str,
) -> anyhow::Result<()> {
    ensure!(
        is_versioned_name(aliased, name),
        "{name} is not a versioned collection of {}",
        aliased.alias
    );
    if !client
        .collection_exists(name)
        .await
        .with_context(|| format!("Qdrant collection {name} existence check failed"))?
    {
        let request = CreateCollectionBuilder::new(name)
            .vectors_config(VectorParamsBuilder::new(
                aliased.dimensions,
                Distance::Cosine,
            ))
            .on_disk_payload(true);
        if let Err(error) = client.create_collection(request).await {
            tracing::warn!(
                collection = name,
                %error,
                "Qdrant collection create failed; validating current state"
            );
        }
    }
    validate_versioned_collection(aliased, name, &load_collection_info(client, name).await?)
}

fn validate_versioned_collection(
    aliased: AliasedCollection,
    name: &str,
    info: &CollectionInfo,
) -> anyhow::Result<()> {
    if let Ok(CollectionStatus::Red) = CollectionStatus::try_from(info.status) {
        bail!("Qdrant collection {name} is unhealthy");
    }
    let vectors = unnamed_vectors(name, info)?;
    ensure!(
        vectors.size == aliased.dimensions,
        "Qdrant collection {name} has {} dimensions, {} requires {}",
        vectors.size,
        aliased.alias,
        aliased.dimensions
    );
    ensure!(
        Distance::try_from(vectors.distance) == Ok(Distance::Cosine),
        "Qdrant collection {name} does not use Cosine distance"
    );
    Ok(())
}

async fn upsert_versioned_points(
    client: &Qdrant,
    aliased: AliasedCollection,
    name: &str,
    points: Vec<(u64, Vec<f32>)>,
) -> anyhow::Result<usize> {
    ensure!(
        is_versioned_name(aliased, name),
        "{name} is not a versioned collection of {}",
        aliased.alias
    );
    for (_, vector) in &points {
        validate_vector(vector, aliased.dimensions, aliased.alias)?;
    }
    if points.is_empty() {
        return Ok(0);
    }
    let points = points
        .into_iter()
        .map(|(id, vector)| {
            let payload = serde_json::Map::from_iter([(
                "sc_track_id".to_owned(),
                serde_json::json!(id.to_string()),
            )]);
            PointStruct::new(id, vector, Payload::from(payload))
        })
        .collect::<Vec<_>>();
    let count = points.len();
    let response = client
        .upsert_points_chunked(
            UpsertPointsBuilder::new(name, points).wait(true),
            VERSIONED_UPSERT_CHUNK,
        )
        .await
        .with_context(|| format!("Qdrant points could not be upserted into {name}"))?;
    ensure_completed(response, "versioned upsert")?;
    Ok(count)
}

async fn point_alias(
    client: &Qdrant,
    aliased: AliasedCollection,
    name: &str,
) -> anyhow::Result<()> {
    ensure!(
        is_versioned_name(aliased, name),
        "{name} is not a versioned collection of {}",
        aliased.alias
    );
    validate_versioned_collection(aliased, name, &load_collection_info(client, name).await?)?;
    let response = client
        .create_alias(CreateAliasBuilder::new(name, aliased.alias))
        .await
        .with_context(|| {
            format!(
                "Qdrant alias {} could not be pointed at {name}",
                aliased.alias
            )
        })?;
    ensure!(
        response.result,
        "Qdrant refused to point alias {} at {name}",
        aliased.alias
    );
    Ok(())
}

async fn drop_versioned_collection(
    client: &Qdrant,
    aliased: AliasedCollection,
    name: &str,
) -> anyhow::Result<()> {
    ensure!(
        is_versioned_name(aliased, name),
        "{name} is not a versioned collection of {}",
        aliased.alias
    );
    ensure!(
        alias_target(client, aliased).await?.as_deref() != Some(name),
        "Qdrant collection {name} is still served through {}",
        aliased.alias
    );
    if client
        .collection_exists(name)
        .await
        .with_context(|| format!("Qdrant collection {name} existence check failed"))?
    {
        client
            .delete_collection(name)
            .await
            .with_context(|| format!("Qdrant collection {name} could not be dropped"))?;
    }
    Ok(())
}

async fn upsert_collab_points(
    client: &Qdrant,
    spec: CollectionSpec,
    model: &str,
    points: Vec<(u64, Vec<f32>)>,
) -> anyhow::Result<usize> {
    ensure!(
        spec.dimensions > 0,
        "collab vector dimensions must be positive"
    );
    ensure!(!model.is_empty(), "collab model name must not be empty");
    let expected = usize::try_from(spec.dimensions).context("collab dimensions are too large")?;
    ensure!(
        points.iter().all(|(_, vector)| {
            vector.len() == expected && vector.iter().all(|value| value.is_finite())
        }),
        "collab points contain an invalid vector"
    );
    if points.is_empty() {
        return Ok(0);
    }

    provision_collection(client, spec).await?;
    let points = points
        .into_iter()
        .map(|(id, vector)| {
            let payload = serde_json::Map::from_iter([
                ("sc_track_id".to_owned(), serde_json::json!(id.to_string())),
                (COLLAB_MODEL_KEY.to_owned(), serde_json::json!(model)),
            ]);
            PointStruct::new(id, vector, Payload::from(payload))
        })
        .collect::<Vec<_>>();
    let count = points.len();
    let response = client
        .upsert_points_chunked(UpsertPointsBuilder::new(spec.name, points).wait(true), 500)
        .await
        .context("Qdrant collab points could not be upserted")?;
    ensure_completed(response, "collab upsert")?;
    Ok(count)
}

async fn remove_points_of_other_models(
    client: &Qdrant,
    collection: &str,
    model: &str,
) -> anyhow::Result<()> {
    ensure!(!model.is_empty(), "collab model name must not be empty");
    let response = client
        .delete_points(
            DeletePointsBuilder::new(collection)
                .points(Filter::must_not([Condition::matches(
                    COLLAB_MODEL_KEY,
                    model.to_owned(),
                )]))
                .wait(true),
        )
        .await
        .with_context(|| {
            format!("Qdrant {collection} points of older models could not be removed")
        })?;
    ensure_completed(response, "collab older model removal")
}

fn ensure_completed(response: PointsOperationResponse, operation: &str) -> anyhow::Result<()> {
    let result = response
        .result
        .with_context(|| format!("Qdrant {operation} returned no result"))?;
    let status = UpdateStatus::try_from(result.status).with_context(|| {
        format!(
            "Qdrant {operation} returned unknown status {}",
            result.status
        )
    })?;
    ensure!(
        status == UpdateStatus::Completed,
        "Qdrant {operation} returned {status:?}"
    );
    Ok(())
}

fn validate_vector(vector: &[f32], dimensions: u64, name: &str) -> anyhow::Result<()> {
    let dimensions = usize::try_from(dimensions).context("vector dimensions are too large")?;
    ensure!(
        vector.len() == dimensions,
        "{name} vector has {} values, expected {dimensions}",
        vector.len()
    );
    ensure!(
        vector.iter().all(|value| value.is_finite()),
        "{name} vector contains a non-finite value"
    );
    Ok(())
}

fn collab_spec(dimensions: u64) -> CollectionSpec {
    CollectionSpec {
        name: TRACKS_COLLAB,
        dimensions,
        profile: CollectionProfile::Search,
    }
}

async fn provision_collection(client: &Qdrant, spec: CollectionSpec) -> anyhow::Result<()> {
    if client
        .collection_exists(spec.name)
        .await
        .with_context(|| format!("Qdrant collection {} existence check failed", spec.name))?
    {
        return validate_loaded_collection(client, spec).await;
    }

    match client.create_collection(create_request(spec)).await {
        Ok(response) if response.result => {
            tracing::info!(
                collection = spec.name,
                dimensions = spec.dimensions,
                "Qdrant collection created"
            );
        }
        Ok(_) => {
            tracing::warn!(
                collection = spec.name,
                "Qdrant collection create returned no result; validating current state"
            );
        }
        Err(error) => {
            tracing::warn!(
                collection = spec.name,
                %error,
                "Qdrant collection create failed; validating current state"
            );
        }
    }

    validate_loaded_collection(client, spec).await
}

async fn validate_loaded_collection(client: &Qdrant, spec: CollectionSpec) -> anyhow::Result<()> {
    let info = load_collection_info(client, spec.name).await?;
    validate_collection(spec, &info)
}

async fn load_collection_info(client: &Qdrant, name: &str) -> anyhow::Result<CollectionInfo> {
    client
        .collection_info(name)
        .await
        .with_context(|| format!("Qdrant collection {name} lookup failed"))?
        .result
        .with_context(|| format!("Qdrant collection {name} returned no configuration"))
}

fn create_request(spec: CollectionSpec) -> CreateCollection {
    let vectors = VectorParamsBuilder::new(spec.dimensions, Distance::Cosine);
    match spec.profile {
        CollectionProfile::Search => CreateCollectionBuilder::new(spec.name)
            .vectors_config(vectors)
            .on_disk_payload(true)
            .build(),
        CollectionProfile::Lookup => CreateCollectionBuilder::new(spec.name)
            .vectors_config(vectors.on_disk(true))
            .hnsw_config(HnswConfigDiffBuilder::default().m(0))
            .on_disk_payload(true)
            .build(),
    }
}

fn validate_collection(spec: CollectionSpec, info: &CollectionInfo) -> anyhow::Result<()> {
    validate_status(spec, info.status)?;
    validate_vectors(spec, unnamed_vectors(spec.name, info)?)
}

fn unnamed_vectors<'a>(name: &str, info: &'a CollectionInfo) -> anyhow::Result<&'a VectorParams> {
    let params = info
        .config
        .as_ref()
        .and_then(|config| config.params.as_ref())
        .with_context(|| format!("Qdrant collection {name} has no parameters"))?;
    let vectors = params
        .vectors_config
        .as_ref()
        .and_then(|config| config.config.as_ref())
        .with_context(|| format!("Qdrant collection {name} has no vector configuration"))?;
    let VectorsConfig::Params(vectors) = vectors else {
        bail!(
            "Qdrant collection {} uses named vectors instead of one unnamed vector",
            name
        );
    };
    Ok(vectors)
}

fn validate_status(spec: CollectionSpec, raw_status: i32) -> anyhow::Result<()> {
    match CollectionStatus::try_from(raw_status) {
        Ok(CollectionStatus::Green | CollectionStatus::Yellow | CollectionStatus::Grey) => Ok(()),
        Ok(CollectionStatus::Red) => bail!("Qdrant collection {} is unhealthy", spec.name),
        Ok(CollectionStatus::UnknownCollectionStatus) | Err(_) => bail!(
            "Qdrant collection {} returned unknown status {raw_status}",
            spec.name
        ),
    }
}

fn validate_vectors(spec: CollectionSpec, vectors: &VectorParams) -> anyhow::Result<()> {
    if vectors.size != spec.dimensions {
        bail!(
            "Qdrant collection {} has {} dimensions, expected {}",
            spec.name,
            vectors.size,
            spec.dimensions
        );
    }
    match Distance::try_from(vectors.distance) {
        Ok(Distance::Cosine) => {}
        Ok(distance) => bail!(
            "Qdrant collection {} uses {distance:?} distance, expected Cosine",
            spec.name
        ),
        Err(_) => bail!(
            "Qdrant collection {} returned unknown distance {}",
            spec.name,
            vectors.distance
        ),
    }
    match vectors.datatype.map(Datatype::try_from) {
        None | Some(Ok(Datatype::Default | Datatype::Float32)) => {}
        Some(Ok(datatype)) => bail!(
            "Qdrant collection {} uses {datatype:?} vectors, expected Float32",
            spec.name
        ),
        Some(Err(_)) => bail!(
            "Qdrant collection {} returned unknown vector datatype",
            spec.name
        ),
    }
    if vectors.multivector_config.is_some() {
        bail!(
            "Qdrant collection {} uses multivectors instead of dense vectors",
            spec.name
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use qdrant_client::qdrant::{
        CollectionConfig, CollectionParams, VectorsConfig as QdrantVectorsConfig,
    };

    use super::*;

    const SPEC: CollectionSpec = CollectionSpec {
        name: "tracks_test",
        dimensions: 512,
        profile: CollectionProfile::Search,
    };

    #[test]
    fn accepts_expected_collection() {
        assert!(
            validate_collection(
                SPEC,
                &collection_info(512, Distance::Cosine, CollectionStatus::Green)
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_incompatible_collection() {
        assert!(
            validate_collection(
                SPEC,
                &collection_info(1024, Distance::Dot, CollectionStatus::Green)
            )
            .is_err()
        );
        assert!(
            validate_collection(
                SPEC,
                &collection_info(512, Distance::Cosine, CollectionStatus::Red)
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_only_completed_point_updates() {
        let completed = PointsOperationResponse {
            result: Some(qdrant_client::qdrant::UpdateResult {
                operation_id: Some(1),
                status: UpdateStatus::Completed as i32,
            }),
            ..Default::default()
        };
        let acknowledged = PointsOperationResponse {
            result: Some(qdrant_client::qdrant::UpdateResult {
                operation_id: Some(2),
                status: UpdateStatus::Acknowledged as i32,
            }),
            ..Default::default()
        };

        assert!(ensure_completed(completed, "test").is_ok());
        assert!(ensure_completed(acknowledged, "test").is_err());
        assert!(ensure_completed(PointsOperationResponse::default(), "test").is_err());
    }

    #[tokio::test]
    #[ignore = "needs the live Qdrant stand"]
    async fn a_new_collab_model_leaves_no_point_of_the_older_ones() -> anyhow::Result<()> {
        let spec = CollectionSpec {
            name: "tracks_collab_live_test",
            dimensions: 4,
            profile: CollectionProfile::Search,
        };
        let client = Qdrant::from_url(
            &std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6334".to_owned()),
        )
        .skip_compatibility_check()
        .build()?;
        if client.collection_exists(spec.name).await? {
            client.delete_collection(spec.name).await?;
        }

        upsert_collab_points(
            &client,
            spec,
            "model-a",
            vec![(1, vec![0.1; 4]), (2, vec![0.2; 4])],
        )
        .await?;
        let legacy = PointStruct::new(
            4,
            vec![0.4; 4],
            Payload::from(serde_json::Map::from_iter([(
                "sc_track_id".to_owned(),
                serde_json::json!("4"),
            )])),
        );
        client
            .upsert_points(UpsertPointsBuilder::new(spec.name, vec![legacy]).wait(true))
            .await?;
        upsert_collab_points(
            &client,
            spec,
            "model-b",
            vec![(2, vec![0.5; 4]), (3, vec![0.3; 4])],
        )
        .await?;
        remove_points_of_other_models(&client, spec.name, "model-b").await?;

        let stored = client
            .get_points(
                GetPointsBuilder::new(spec.name, [1_u64, 2, 3, 4].map(PointId::from).to_vec())
                    .with_payload(true),
            )
            .await?;
        client.delete_collection(spec.name).await?;
        let mut kept = stored
            .result
            .into_iter()
            .filter_map(|point| match point.id?.point_id_options? {
                PointIdOptions::Num(id) => Some((
                    id,
                    point
                        .payload
                        .get(COLLAB_MODEL_KEY)
                        .and_then(|value| value.as_str().cloned()),
                )),
                PointIdOptions::Uuid(_) => None,
            })
            .collect::<Vec<_>>();
        kept.sort_unstable();

        assert_eq!(
            kept,
            vec![
                (2, Some("model-b".to_owned())),
                (3, Some("model-b".to_owned()))
            ]
        );
        Ok(())
    }

    const LIVE_TASTE: AliasedCollection = AliasedCollection {
        alias: "tracks_taste_live_alias",
        prefix: "tracks_taste_live_",
        dimensions: 4,
    };

    #[test]
    fn only_prefixed_version_collections_belong_to_an_alias() {
        assert!(is_versioned_name(
            TRACKS_TASTE,
            "tracks_taste_202609241200_0a1b2c3d"
        ));
        assert!(!is_versioned_name(TRACKS_TASTE, "tracks_taste"));
        assert!(!is_versioned_name(TRACKS_TASTE, "tracks_taste_"));
        assert!(!is_versioned_name(TRACKS_TASTE, "tracks_collab"));
        assert!(!is_versioned_name(TRACKS_TASTE, "tracks_taste_Latest"));
        assert!(
            REQUIRED_COLLECTIONS.iter().all(|spec| ALIASED_COLLECTIONS
                .iter()
                .all(
                    |aliased| spec.name != aliased.alias && !is_versioned_name(*aliased, spec.name)
                )),
            "provisioning a required collection under an alias name would make the alias impossible"
        );
    }

    #[test]
    fn a_version_collection_must_match_the_alias_space() {
        let name = "tracks_taste_202609241200_0a1b2c3d";

        assert!(
            validate_versioned_collection(
                TRACKS_TASTE,
                name,
                &collection_info(128, Distance::Cosine, CollectionStatus::Green)
            )
            .is_ok()
        );
        assert!(
            validate_versioned_collection(
                TRACKS_TASTE,
                name,
                &collection_info(512, Distance::Cosine, CollectionStatus::Green)
            )
            .is_err()
        );
        assert!(
            validate_versioned_collection(
                TRACKS_TASTE,
                name,
                &collection_info(128, Distance::Dot, CollectionStatus::Green)
            )
            .is_err()
        );
    }

    #[tokio::test]
    #[ignore = "needs the live Qdrant stand"]
    async fn the_alias_moves_to_a_new_version_at_once_and_the_old_one_can_then_go()
    -> anyhow::Result<()> {
        let client = Qdrant::from_url(
            &std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6334".to_owned()),
        )
        .skip_compatibility_check()
        .build()?;
        let first = "tracks_taste_live_202609231200_00000001";
        let second = "tracks_taste_live_202609241200_00000002";
        if alias_target(&client, LIVE_TASTE).await?.is_some() {
            client.delete_alias(LIVE_TASTE.alias).await?;
        }
        for name in [first, second] {
            if client.collection_exists(name).await? {
                client.delete_collection(name).await?;
            }
        }

        ensure_versioned_collection(&client, LIVE_TASTE, first).await?;
        upsert_versioned_points(
            &client,
            LIVE_TASTE,
            first,
            vec![(1, vec![1.0, 0.0, 0.0, 0.0])],
        )
        .await?;
        point_alias(&client, LIVE_TASTE, first).await?;
        validate_alias_target(&client, LIVE_TASTE).await?;
        ensure_versioned_collection(&client, LIVE_TASTE, second).await?;
        upsert_versioned_points(
            &client,
            LIVE_TASTE,
            second,
            vec![(2, vec![0.0, 1.0, 0.0, 0.0]), (3, vec![0.0, 0.0, 1.0, 0.0])],
        )
        .await?;
        let refused_while_served = drop_versioned_collection(&client, LIVE_TASTE, first).await;
        point_alias(&client, LIVE_TASTE, second).await?;
        let target = alias_target(&client, LIVE_TASTE).await?;
        let through_alias = client
            .count(CountPointsBuilder::new(LIVE_TASTE.alias).exact(true))
            .await?
            .result
            .map(|counted| counted.count);
        drop_versioned_collection(&client, LIVE_TASTE, first).await?;
        let first_left = client.collection_exists(first).await?;
        let wrong_space =
            upsert_versioned_points(&client, LIVE_TASTE, second, vec![(4, vec![1.0, 0.0])]).await;

        client.delete_alias(LIVE_TASTE.alias).await?;
        client.delete_collection(second).await?;

        assert!(refused_while_served.is_err());
        assert_eq!(target.as_deref(), Some(second));
        assert_eq!(through_alias, Some(2));
        assert!(!first_left);
        assert!(wrong_space.is_err());
        Ok(())
    }

    fn collection_info(
        dimensions: u64,
        distance: Distance,
        status: CollectionStatus,
    ) -> CollectionInfo {
        CollectionInfo {
            status: status as i32,
            config: Some(CollectionConfig {
                params: Some(CollectionParams {
                    vectors_config: Some(QdrantVectorsConfig {
                        config: Some(VectorsConfig::Params(VectorParams {
                            size: dimensions,
                            distance: distance as i32,
                            ..Default::default()
                        })),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}
