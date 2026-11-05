use serde::Deserialize;

use super::Seerr;
use lemonfiber_ports::http::Method;
use lemonfiber_ports::service::{Endpoint, Failure, MediaServerLink};

const LINK: &str = "/settings/jellyfin";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkResource {
    #[serde(default)]
    ip: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    url_base: String,
    #[serde(default)]
    api_key: String,
}

pub(super) async fn media_server_link(seerr: &Seerr) -> Result<MediaServerLink, Failure> {
    let response = seerr.send(Method::Get, LINK, None).await?;
    let link: LinkResource = seerr
        .endpoint
        .decode(&response, "Seerr's media-server settings could not be read")?;
    Ok(MediaServerLink {
        at: Endpoint {
            host: link.ip,
            port: link.port,
            base: link.url_base,
        },
        key: link.api_key,
    })
}

pub(super) async fn link_media_server(
    seerr: &Seerr,
    at: &Endpoint,
    key: &str,
) -> Result<(), Failure> {
    let body = serde_json::json!({
        "ip": at.host,
        "port": at.port,
        "useSsl": false,
        "urlBase": at.base,
        "apiKey": key,
    })
    .to_string();
    let written = seerr.send(Method::Post, LINK, Some(body)).await?;
    seerr.endpoint.expect_success(&written)
}
