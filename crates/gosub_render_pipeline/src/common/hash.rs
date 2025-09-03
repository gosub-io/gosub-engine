use sha2::{Digest, Sha256};

pub type Sha256Hash = [u8; 32];

pub fn hash_from_string(data: &str) -> Sha256Hash {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    hasher.finalize().into()
}

pub fn hash_from_data(data: &[u8]) -> Sha256Hash {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}