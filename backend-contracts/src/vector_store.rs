use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionProfile {
    Search,
    Lookup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectionSpec {
    pub name: &'static str,
    pub dimensions: u64,
    pub profile: CollectionProfile,
}

pub const TRACKS_MERT: &str = "tracks_mert";
pub const TRACKS_MERT_DIMENSIONS: u64 = 1024;
pub const TRACKS_CLAP: &str = "tracks_clap";
pub const TRACKS_CLAP_DIMENSIONS: u64 = 512;
pub const TRACKS_LYRICS: &str = "tracks_lyrics";
pub const TRACKS_LYRICS_DIMENSIONS: u64 = 1024;
pub const TRACKS_COLLAB: &str = "tracks_collab";
pub const TRACKS_COLLAB_DIMENSIONS: u64 = 128;
pub const TRACKS_TASTE_DIMENSIONS: u64 = 128;
pub const QUERY_VEC_MULAN: &str = "query_vectors_mulan";
pub const QUERY_VEC_MULAN_DIMENSIONS: u64 = 512;
pub const QUERY_VEC_LYRICS: &str = "query_vectors_lyrics";
pub const QUERY_VEC_LYRICS_DIMENSIONS: u64 = 1024;

pub const REQUIRED_COLLECTIONS: [CollectionSpec; 6] = [
    CollectionSpec {
        name: TRACKS_MERT,
        dimensions: TRACKS_MERT_DIMENSIONS,
        profile: CollectionProfile::Search,
    },
    CollectionSpec {
        name: TRACKS_CLAP,
        dimensions: TRACKS_CLAP_DIMENSIONS,
        profile: CollectionProfile::Search,
    },
    CollectionSpec {
        name: TRACKS_LYRICS,
        dimensions: TRACKS_LYRICS_DIMENSIONS,
        profile: CollectionProfile::Search,
    },
    CollectionSpec {
        name: TRACKS_COLLAB,
        dimensions: TRACKS_COLLAB_DIMENSIONS,
        profile: CollectionProfile::Search,
    },
    CollectionSpec {
        name: QUERY_VEC_MULAN,
        dimensions: QUERY_VEC_MULAN_DIMENSIONS,
        profile: CollectionProfile::Lookup,
    },
    CollectionSpec {
        name: QUERY_VEC_LYRICS,
        dimensions: QUERY_VEC_LYRICS_DIMENSIONS,
        profile: CollectionProfile::Lookup,
    },
];

pub fn query_point_uuid(hash: &str) -> String {
    let canonical = hash.len() >= 32 && hash.as_bytes()[..32].iter().all(u8::is_ascii_hexdigit);
    let fallback;
    let head: &str = if canonical {
        &hash[..32]
    } else {
        fallback = hex::encode(Sha256::digest(hash.as_bytes()));
        &fallback[..32]
    };
    format!(
        "{}-{}-{}-{}-{}",
        &head[0..8],
        &head[8..12],
        &head[12..16],
        &head[16..20],
        &head[20..32]
    )
}

pub fn query_vector_collection(model: &str) -> Option<&'static str> {
    match model {
        "mulan" => Some(QUERY_VEC_MULAN),
        "lyrics" => Some(QUERY_VEC_LYRICS),
        _ => None,
    }
}
