#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    #[error("the envelope is malformed")]
    Envelope,
    #[error("envelope version {0} is not supported")]
    Version(u8),
    #[error("the envelope is larger than 64 KB")]
    TooLarge,
    #[error("a key is malformed")]
    Key,
    #[error("sealing failed")]
    Seal,
    #[error("the ciphertext does not open")]
    Open,
    #[error("the signature does not verify")]
    Signature,
    #[error("the protobuf does not decode")]
    Decode,
    #[error("the system has no randomness")]
    Random,
}
