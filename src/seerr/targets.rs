use serde::Deserialize;

use super::Seerr;
use lemonfiber_ports::http::Method;
use lemonfiber_ports::media::Kind;
use lemonfiber_ports::service::{Endpoint, Failure, FulfilmentTarget, RegisteredTarget};

const fn list(kind: Kind) -> &'static str {
    match kind {
        Kind::Movies => "/settings/radarr",
        Kind::Tv => "/settings/sonarr",
    }
}

const WHEN_RELEASED: &str = "released";

const ID: &str = "id";

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
        ("activeProfileId", serde_json::json!(target.profile.id)),
        ("activeProfileName", serde_json::json!(target.profile.name)),
        ("activeDirectory", serde_json::json!(target.folder)),
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

pub(super) async fn move_fulfilment_target(
    seerr: &Seerr,
    held: &RegisteredTarget,
    at: &Endpoint,
    key: &str,
) -> Result<(), Failure> {
    seerr.held_to_an_address(at)?;
    let path = list(held.kind);
    let response = seerr.send(Method::Get, path, None).await?;
    let listed: Vec<serde_json::Map<String, serde_json::Value>> = seerr
        .endpoint
        .decode(&response, "Seerr's fulfilment targets could not be read")?;
    let Some(mut moved) = listed.into_iter().find(|one| {
        one.get(ID)
            .and_then(serde_json::Value::as_i64)
            .is_some_and(|id| id.to_string() == held.id)
    }) else {
        return Err(seerr.endpoint.refused(&format!(
            "Seerr no longer holds the target it listed as {}",
            held.id
        )));
    };
    moved.remove(ID);
    for (at, value) in [
        ("hostname", serde_json::json!(at.host)),
        ("port", serde_json::json!(at.port)),
        ("baseUrl", serde_json::json!(at.base)),
        ("apiKey", serde_json::json!(key)),
        ("useSsl", serde_json::json!(false)),
    ] {
        moved.insert(at.to_owned(), value);
    }
    let body = serde_json::Value::Object(moved).to_string();
    let written = seerr
        .send(Method::Put, &format!("{path}/{}", held.id), Some(body))
        .await?;
    seerr.endpoint.expect_success(&written)
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
