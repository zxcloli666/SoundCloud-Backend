use axum::Json;
use axum::extract::{FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

use crate::modules::chat::ChatError;

pub struct ChatJson<T>(pub T);

impl<T, S> FromRequest<S> for ChatJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ChatError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| ChatJson(value))
            .map_err(|_| ChatError::Invalid)
    }
}

pub struct ChatPath<T>(pub T);

impl<T, S> FromRequestParts<S> for ChatPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ChatError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| ChatPath(value))
            .map_err(|_| ChatError::NotFound)
    }
}

pub struct ChatQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ChatQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ChatError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| ChatQuery(value))
            .map_err(|_| ChatError::Invalid)
    }
}
