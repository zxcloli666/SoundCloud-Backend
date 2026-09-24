use backend_contracts::vector_store::{CollectionSpec, REQUIRED_COLLECTIONS};
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{
    CollectionInfo, CollectionStatus, Datatype, Distance, VectorParams,
    vectors_config::Config as VectorsConfig,
};

use crate::error::{AppError, AppResult};

pub(super) async fn prepare_required_collections(client: &Qdrant) -> AppResult<()> {
    for spec in REQUIRED_COLLECTIONS {
        prepare_collection(client, spec)
            .await
            .map_err(AppError::internal)?;
    }
    Ok(())
}

async fn prepare_collection(client: &Qdrant, spec: CollectionSpec) -> Result<(), String> {
    let exists = client.collection_exists(spec.name).await.map_err(|error| {
        format!(
            "Qdrant collection {} existence check failed: {error}",
            spec.name
        )
    })?;

    if exists {
        return load_and_validate(client, spec).await;
    }
    Err(format!(
        "required Qdrant collection {} does not exist",
        spec.name
    ))
}

async fn load_and_validate(client: &Qdrant, spec: CollectionSpec) -> Result<(), String> {
    let response = client
        .collection_info(spec.name)
        .await
        .map_err(|error| format!("Qdrant collection {} lookup failed: {error}", spec.name))?;
    let info = response
        .result
        .ok_or_else(|| format!("Qdrant collection {} returned no configuration", spec.name))?;
    validate_collection(spec, &info)
}

fn validate_collection(spec: CollectionSpec, info: &CollectionInfo) -> Result<(), String> {
    validate_status(spec, info.status)?;
    let params = info
        .config
        .as_ref()
        .and_then(|config| config.params.as_ref())
        .ok_or_else(|| {
            format!(
                "Qdrant collection {} has no collection parameters",
                spec.name
            )
        })?;
    let vectors = params
        .vectors_config
        .as_ref()
        .and_then(|config| config.config.as_ref())
        .ok_or_else(|| {
            format!(
                "Qdrant collection {} has no vector configuration",
                spec.name
            )
        })?;
    let vectors = match vectors {
        VectorsConfig::Params(vectors) => vectors,
        VectorsConfig::ParamsMap(_) => {
            return Err(format!(
                "Qdrant collection {} uses named vectors instead of one unnamed vector",
                spec.name
            ));
        }
    };
    validate_vectors(spec, vectors)
}

fn validate_status(spec: CollectionSpec, raw_status: i32) -> Result<(), String> {
    match CollectionStatus::try_from(raw_status) {
        Ok(CollectionStatus::Green | CollectionStatus::Yellow | CollectionStatus::Grey) => Ok(()),
        Ok(CollectionStatus::Red) => Err(format!("Qdrant collection {} is unhealthy", spec.name)),
        Ok(CollectionStatus::UnknownCollectionStatus) | Err(_) => Err(format!(
            "Qdrant collection {} returned unknown status {raw_status}",
            spec.name
        )),
    }
}

fn validate_vectors(spec: CollectionSpec, vectors: &VectorParams) -> Result<(), String> {
    if vectors.size != spec.dimensions {
        return Err(format!(
            "Qdrant collection {} has {} dimensions, expected {}",
            spec.name, vectors.size, spec.dimensions
        ));
    }
    match Distance::try_from(vectors.distance) {
        Ok(Distance::Cosine) => {}
        Ok(distance) => {
            return Err(format!(
                "Qdrant collection {} uses {distance:?} distance, expected Cosine",
                spec.name
            ));
        }
        Err(_) => {
            return Err(format!(
                "Qdrant collection {} returned unknown distance {}",
                spec.name, vectors.distance
            ));
        }
    }
    match vectors.datatype {
        None => {}
        Some(datatype) => match Datatype::try_from(datatype) {
            Ok(Datatype::Default | Datatype::Float32) => {}
            Ok(datatype) => {
                return Err(format!(
                    "Qdrant collection {} uses {datatype:?} vectors, expected Float32",
                    spec.name
                ));
            }
            Err(_) => {
                return Err(format!(
                    "Qdrant collection {} returned unknown vector datatype {datatype}",
                    spec.name
                ));
            }
        },
    }
    if vectors.multivector_config.is_some() {
        return Err(format!(
            "Qdrant collection {} uses multivectors instead of dense vectors",
            spec.name
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use backend_contracts::vector_store::CollectionProfile;
    use qdrant_client::qdrant::{
        CollectionConfig, CollectionParams, MultiVectorConfig, VectorParamsMap,
        VectorsConfig as QdrantVectorsConfig,
    };

    use super::*;

    const SPEC: CollectionSpec = CollectionSpec {
        name: "tracks_test",
        dimensions: 512,
        profile: CollectionProfile::Search,
    };

    #[test]
    fn accepts_compatible_collection_states_and_float_types() {
        for status in [
            CollectionStatus::Green,
            CollectionStatus::Yellow,
            CollectionStatus::Grey,
        ] {
            for datatype in [None, Some(Datatype::Default), Some(Datatype::Float32)] {
                let info = collection_info(status, vector_params(512, Distance::Cosine, datatype));
                assert!(validate_collection(SPEC, &info).is_ok());
            }
        }
    }

    #[test]
    fn rejects_wrong_dimensions() {
        let info = collection_info(
            CollectionStatus::Green,
            vector_params(1024, Distance::Cosine, None),
        );
        assert!(validate_collection(SPEC, &info).is_err());
    }

    #[test]
    fn rejects_wrong_distance() {
        let info = collection_info(
            CollectionStatus::Green,
            vector_params(512, Distance::Dot, None),
        );
        assert!(validate_collection(SPEC, &info).is_err());
    }

    #[test]
    fn rejects_named_vectors() {
        let vectors = QdrantVectorsConfig {
            config: Some(VectorsConfig::ParamsMap(VectorParamsMap {
                map: HashMap::from([(
                    "dense".to_owned(),
                    vector_params(512, Distance::Cosine, None),
                )]),
            })),
        };
        let info = collection_info_with_config(CollectionStatus::Green, Some(vectors));
        assert!(validate_collection(SPEC, &info).is_err());
    }

    #[test]
    fn rejects_missing_vector_configuration() {
        let info = collection_info_with_config(CollectionStatus::Green, None);
        assert!(validate_collection(SPEC, &info).is_err());
    }

    #[test]
    fn rejects_red_and_unknown_statuses() {
        let vectors = vector_params(512, Distance::Cosine, None);
        let red = collection_info(CollectionStatus::Red, vectors);
        let unknown = CollectionInfo {
            status: 99,
            ..red.clone()
        };
        assert!(validate_collection(SPEC, &red).is_err());
        assert!(validate_collection(SPEC, &unknown).is_err());
    }

    #[test]
    fn rejects_non_float_and_multivector_configurations() {
        let uint8 = collection_info(
            CollectionStatus::Green,
            vector_params(512, Distance::Cosine, Some(Datatype::Uint8)),
        );
        let mut multivector = vector_params(512, Distance::Cosine, None);
        multivector.multivector_config = Some(MultiVectorConfig::default());
        let multivector = collection_info(CollectionStatus::Green, multivector);
        assert!(validate_collection(SPEC, &uint8).is_err());
        assert!(validate_collection(SPEC, &multivector).is_err());
    }

    fn vector_params(
        dimensions: u64,
        distance: Distance,
        datatype: Option<Datatype>,
    ) -> VectorParams {
        VectorParams {
            size: dimensions,
            distance: distance as i32,
            datatype: datatype.map(|datatype| datatype as i32),
            ..Default::default()
        }
    }

    fn collection_info(status: CollectionStatus, vectors: VectorParams) -> CollectionInfo {
        collection_info_with_config(
            status,
            Some(QdrantVectorsConfig {
                config: Some(VectorsConfig::Params(vectors)),
            }),
        )
    }

    fn collection_info_with_config(
        status: CollectionStatus,
        vectors_config: Option<QdrantVectorsConfig>,
    ) -> CollectionInfo {
        CollectionInfo {
            status: status as i32,
            config: Some(CollectionConfig {
                params: Some(CollectionParams {
                    vectors_config,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod live_tests {
    use backend_contracts::vector_store::REQUIRED_COLLECTIONS;
    use qdrant_client::qdrant::{CreateCollectionBuilder, Distance, VectorParamsBuilder};

    use super::prepare_required_collections;
    use crate::config::QdrantCfg;
    use crate::qdrant::QdrantService;

    fn service() -> std::sync::Arc<QdrantService> {
        QdrantService::connect(&QdrantCfg {
            grpc_url: std::env::var("QDRANT_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:6334".to_owned()),
            api_key: String::new().into(),
        })
        .expect("the client builds")
    }

    async fn provision_as_declared(service: &QdrantService) -> anyhow::Result<()> {
        for spec in REQUIRED_COLLECTIONS {
            let _ = service.raw().delete_collection(spec.name).await;
            service
                .raw()
                .create_collection(
                    CreateCollectionBuilder::new(spec.name).vectors_config(
                        VectorParamsBuilder::new(spec.dimensions, Distance::Cosine),
                    ),
                )
                .await?;
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a local Qdrant"]
    async fn a_store_provisioned_as_declared_is_accepted() -> anyhow::Result<()> {
        let service = service();
        provision_as_declared(&service).await?;

        prepare_required_collections(service.raw())
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a local Qdrant"]
    async fn a_collection_of_the_wrong_width_stops_the_service_from_starting() -> anyhow::Result<()>
    {
        let service = service();
        provision_as_declared(&service).await?;
        let wrong = REQUIRED_COLLECTIONS[0];

        let _ = service.raw().delete_collection(wrong.name).await;
        service
            .raw()
            .create_collection(CreateCollectionBuilder::new(wrong.name).vectors_config(
                VectorParamsBuilder::new(wrong.dimensions + 8, Distance::Cosine),
            ))
            .await?;
        let refused = prepare_required_collections(service.raw()).await;
        provision_as_declared(&service).await?;

        let error = refused.expect_err("a collection of the wrong width must stop the start");
        assert!(
            error.to_string().contains(wrong.name),
            "the refusal must name the collection so whoever runs it knows what to fix: {error}"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a local Qdrant"]
    async fn a_missing_collection_stops_the_service_from_starting() -> anyhow::Result<()> {
        let service = service();
        provision_as_declared(&service).await?;
        let absent = REQUIRED_COLLECTIONS[REQUIRED_COLLECTIONS.len() - 1];

        let _ = service.raw().delete_collection(absent.name).await;
        let refused = prepare_required_collections(service.raw()).await;
        provision_as_declared(&service).await?;

        let error = refused.expect_err("a missing collection must stop the start");
        assert!(
            error.to_string().contains(absent.name),
            "the refusal must name the collection that is not there: {error}"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a local Qdrant"]
    async fn a_collection_measured_by_the_wrong_distance_is_refused() -> anyhow::Result<()> {
        let service = service();
        provision_as_declared(&service).await?;
        let wrong = REQUIRED_COLLECTIONS[1];

        let _ = service.raw().delete_collection(wrong.name).await;
        service
            .raw()
            .create_collection(
                CreateCollectionBuilder::new(wrong.name)
                    .vectors_config(VectorParamsBuilder::new(wrong.dimensions, Distance::Dot)),
            )
            .await?;
        let refused = prepare_required_collections(service.raw()).await;
        provision_as_declared(&service).await?;

        let error = refused.expect_err("cosine is what every score in this service assumes");
        assert!(error.to_string().contains(wrong.name), "{error}");
        Ok(())
    }
}
