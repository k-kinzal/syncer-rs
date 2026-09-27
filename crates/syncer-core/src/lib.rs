//! Layered policy resolution, async extension access and safe file plans.
pub mod engine;
pub mod extension;
pub mod policy;
pub mod report;
pub mod storage;
pub use engine::{Context, Plan, plan};
pub use extension::Extensions;
pub use policy::{Layer, Resolved, resolve};
pub fn digest(data: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(data))
}
