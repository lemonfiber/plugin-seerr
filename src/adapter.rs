//! What this adapter says it is, and the kit that serves its contract.

use std::sync::Arc;

use lemonfiber_adapter_kit::{Kit, Readiness};
use lemonfiber_contract::adapter::{About, Release};
use lemonfiber_contract::capabilities::request;

use crate::seerr::Seerr;

/// The upstream this adapter fronts, as its project names itself.
pub const UPSTREAM: &str = "Seerr";

/// The Seerr release this adapter's recordings were taken from.
pub const RELEASE: &str = "v3.5.0";

/// The digest of the image that release's recordings were taken from.
pub const DIGEST: &str = "sha256:27602401178d54f1964442287b9f23f67a3fa2252645ee8065839ea1c3f69e45";

/// What this adapter says it is: the contract it speaks and the release it supports.
#[must_use]
pub fn about() -> About {
    About {
        speaks: vec![lemonfiber_contract::spoken(
            request::intake::CAPABILITY,
            request::intake::MAJOR,
        )],
        upstream: UPSTREAM.to_owned(),
        releases: vec![Release {
            version: RELEASE.to_owned(),
            digest: DIGEST.to_owned(),
        }],
    }
}

/// The contract, served from `seerr` under `key`, ready while Seerr answers.
#[must_use]
pub fn kit(key: String, seerr: Arc<Seerr>) -> Kit {
    let asked = Arc::clone(&seerr);
    let ready: Readiness = Arc::new(move || {
        let asked = Arc::clone(&asked);
        Box::pin(async move { asked.answers().await })
    });
    Kit::new(key, about())
        .ready_when(ready)
        .serving(request::intake::served(seerr))
}

#[cfg(test)]
mod tests;
