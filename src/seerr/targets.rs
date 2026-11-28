use serde::Deserialize;

use super::Seerr;
use lemonfiber_ports::http::Method;
use lemonfiber_ports::media::Kind;
use lemonfiber_ports::service::{
    Endpoint, Failure, FulfilmentTarget, QualityProfile, RegisteredTarget, TargetFiling,
};

const fn list(kind: Kind) -> &'static str {
    match kind {
        Kind::Movies => "/settings/radarr",
        Kind::Tv => "/settings/sonarr",
    }
}

const WHEN_RELEASED: &str = "released";

const ID: &str = "id";

const PROFILE_ID: &str = "activeProfileId";

const PROFILE_NAME: &str = "activeProfileName";

const DIRECTORY: &str = "activeDirectory";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetResource {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    hostname: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_key: String,
}

impl TargetResource {
    fn registered(self, kind: Kind) -> RegisteredTarget {
        RegisteredTarget {
            id: self.id.to_string(),
            at: Endpoint {
                host: self.hostname,
                port: self.port,
                base: self.base_url,
            },
            key: self.api_key,
            kind,
        }
    }
}

pub(super) async fn fulfilment_targets(seerr: &Seerr) -> Result<Vec<RegisteredTarget>, Failure> {
    let mut held = Vec::new();
    for kind in [Kind::Movies, Kind::Tv] {
        let response = seerr.send(Method::Get, list(kind), None).await?;
        let listed: Vec<TargetResource> = seerr
            .endpoint
            .decode(&response, "Seerr's fulfilment targets could not be read")?;
        held.extend(listed.into_iter().map(|target| target.registered(kind)));
    }
    Ok(held)
}

pub(super) async fn add_fulfilment_target(
    seerr: &Seerr,
    target: &FulfilmentTarget,
) -> Result<(), Failure> {
    seerr.held_to_an_address(&target.at)?;
    let differs = if target.kind == Kind::Tv {
        ("enableSeasonFolders", serde_json::json!(true))
    } else {
        ("minimumAvailability", serde_json::json!(WHEN_RELEASED))
    };

    let fields: serde_json::Map<String, serde_json::Value> = [
        ("name", serde_json::json!(target.name)),
        ("hostname", serde_json::json!(target.at.host)),
        ("port", serde_json::json!(target.at.port)),
        ("baseUrl", serde_json::json!(target.at.base)),
        ("apiKey", serde_json::json!(target.key)),
        ("useSsl", serde_json::json!(false)),
        (PROFILE_ID, serde_json::json!(target.profile.id)),
        (PROFILE_NAME, serde_json::json!(target.profile.name)),
        (DIRECTORY, serde_json::json!(target.folder)),
        ("is4k", serde_json::json!(false)),
        ("isDefault", serde_json::json!(true)),
        differs,
    ]
    .into_iter()
    .map(|(at, value)| (at.to_owned(), value))
    .collect();

    let body = serde_json::Value::Object(fields).to_string();
    let path = list(target.kind);
    let written = seerr.send(Method::Post, path, Some(body)).await?;
    seerr.endpoint.expect_success(&written)
}

async fn held_as(
    seerr: &Seerr,
    held: &RegisteredTarget,
) -> Result<serde_json::Map<String, serde_json::Value>, Failure> {
    let response = seerr.send(Method::Get, list(held.kind), None).await?;
    let listed: Vec<serde_json::Map<String, serde_json::Value>> = seerr
        .endpoint
        .decode(&response, "Seerr's fulfilment targets could not be read")?;
    listed
        .into_iter()
        .find(|one| {
            one.get(ID)
                .and_then(serde_json::Value::as_i64)
                .is_some_and(|id| id.to_string() == held.id)
        })
        .ok_or_else(|| {
            seerr.endpoint.refused(&format!(
                "Seerr no longer holds the target it listed as {}",
                held.id
            ))
        })
}

async fn rewritten<const N: usize>(
    seerr: &Seerr,
    held: &RegisteredTarget,
    fields: [(&str, serde_json::Value); N],
) -> Result<(), Failure> {
    let mut target = held_as(seerr, held).await?;
    target.remove(ID);
    for (field, value) in fields {
        target.insert(field.to_owned(), value);
    }
    let body = serde_json::Value::Object(target).to_string();
    let written = seerr
        .send(
            Method::Put,
            &format!("{}/{}", list(held.kind), held.id),
            Some(body),
        )
        .await?;
    seerr.endpoint.expect_success(&written)
}

pub(super) async fn move_fulfilment_target(
    seerr: &Seerr,
    held: &RegisteredTarget,
    at: &Endpoint,
    key: &str,
) -> Result<(), Failure> {
    seerr.held_to_an_address(at)?;
    rewritten(
        seerr,
        held,
        [
            ("hostname", serde_json::json!(at.host)),
            ("port", serde_json::json!(at.port)),
            ("baseUrl", serde_json::json!(at.base)),
            ("apiKey", serde_json::json!(key)),
            ("useSsl", serde_json::json!(false)),
        ],
    )
    .await
}

pub(super) async fn fulfilment_target_filing(
    seerr: &Seerr,
    held: &RegisteredTarget,
) -> Result<TargetFiling, Failure> {
    let found = held_as(seerr, held).await?;
    let unnamed = || {
        seerr.endpoint.refused(&format!(
            "Seerr names no quality profile or root folder for the target it listed as {}",
            held.id
        ))
    };
    let profile = found
        .get(PROFILE_ID)
        .and_then(serde_json::Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .ok_or_else(unnamed)?;
    let folder = found
        .get(DIRECTORY)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(unnamed)?;
    let name = found
        .get(PROFILE_NAME)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    Ok(TargetFiling {
        profile: QualityProfile {
            id: profile,
            name: name.to_owned(),
        },
        folder: folder.to_owned(),
    })
}

pub(super) async fn refile_fulfilment_target(
    seerr: &Seerr,
    held: &RegisteredTarget,
    filing: &TargetFiling,
) -> Result<(), Failure> {
    rewritten(
        seerr,
        held,
        [
            (PROFILE_ID, serde_json::json!(filing.profile.id)),
            (PROFILE_NAME, serde_json::json!(filing.profile.name)),
            (DIRECTORY, serde_json::json!(filing.folder)),
        ],
    )
    .await
}

pub(super) async fn test_fulfilment_target(
    seerr: &Seerr,
    kind: Kind,
    at: &Endpoint,
    key: &str,
) -> Result<(), Failure> {
    seerr.held_to_an_address(at)?;
    let path = list(kind);
    let body = serde_json::json!({
        "hostname": at.host,
        "port": at.port,
        "baseUrl": at.base,
        "apiKey": key,
        "useSsl": false,
    })
    .to_string();
    let tested = seerr
        .send(Method::Post, &format!("{path}/test"), Some(body))
        .await?;
    seerr.endpoint.expect_success(&tested)
}
