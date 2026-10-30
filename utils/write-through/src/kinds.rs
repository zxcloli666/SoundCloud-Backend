use std::fmt::Debug;

use serde::Serialize;
use serde::de::DeserializeOwned;

pub trait Kind: Serialize + Debug + Sync {
    const NAME: &'static str;
    type Output: DeserializeOwned + Send + Unpin + 'static;
}
