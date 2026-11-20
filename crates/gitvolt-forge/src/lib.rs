//! Forge integrations. #1: Gravatar avatars with a disk cache. #4: the forge providers (GitLab,
//! GitHub, Forgejo) over personal access tokens, implementing `gitvolt_core::forge`'s contract.

pub mod gravatar;

pub mod tokens;
pub mod avatar_cache;
pub mod endpoints;
pub mod http;
pub mod images;
pub mod known_names;
pub mod time;
pub mod pipelines;
/// GitVolt's own releases (the update check).
pub mod updates;
#[cfg(test)]
pub(crate) mod test_server;
/// The contract these implement (core's, re-exported for callers of this crate).
pub use gitvolt_core::forge;
pub mod gitlab;
pub mod github;
pub mod forgejo;
pub mod connector;
pub mod detect;
