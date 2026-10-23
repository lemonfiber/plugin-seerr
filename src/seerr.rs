//! Seerr, the request service, asked through its own API with the key lemonfiber wrote for
//! this adapter.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use lemonfiber_ports::http::{Http, Method, Request, Response, JSON};
use lemonfiber_ports::media::Kind;
use lemonfiber_ports::service::{
    Credential, Failure, FulfilmentTarget, HouseholdRequest, IdentitySource, MediaServerLink,
    Occasion, RegisteredTarget, Requesting, Requests, Telling,
};
use serde::Deserialize;

use crate::credential::Holding;
use crate::endpoint::Endpoint;

mod addressing;
mod asking;
mod linking;
mod members;
mod notices;
mod records;
mod searching;
mod targets;

use members::{approves_own, MemberResource, LINK_MEMBERS, MEMBERS, NOT_FOUND};
use records::{RequestPage, RequestRecord, REQUEST_PAGE};

/// What every failure is blamed on.
pub const SERVICE: &str = "seerr";

/// The identity source Seerr signs a household in through, as the stack names its API.
pub const JELLYFIN: &str = "jellyfin";

const API: &str = "/api/v1";

const KEY_HEADER: &str = "X-Api-Key";

const ACTING_AS: &str = "X-API-User";

const CONTENT_TYPE: &str = "Content-Type";

const STATUS: &str = "/status";

const PLAIN: u16 = 80;

const SECURE: u16 = 443;

const OWNER_EMAIL: &str = "admin@lemonfiber.local";

const JELLYFIN_SERVER_TYPE: u8 = 2;

const OCCASION_BITS: [(Occasion, u32); 6] = [
    (Occasion::Received, 2),
    (Occasion::Approved, 4),
    (Occasion::Arrived, 8),
    (Occasion::Failed, 16),
    (Occasion::Declined, 64),
    (Occasion::ApprovedByPolicy, 128),
];

const LONGEST_NUMBER: usize = 19;

const IDENTITY_DIGITS: usize = 32;

const IDENTITY_DASHES: usize = 4;

const WEBPUSH: &str = "/settings/notifications/webpush";

const LONGEST_HOST: usize = 253;

const BASE_MARKS: &[u8] = b"-._~/";

const NOT_AN_ADDRESS: &str =
    "that is not an address Seerr can be told: a host, a port and a plain path";

struct Reached<'a> {
    secure: bool,
    host: &'a str,
    port: u16,
    base: String,
}

fn taken_apart(server_url: &str) -> Option<Reached<'_>> {
    let (secure, rest) = server_url
        .strip_prefix("https://")
        .map(|rest| (true, rest))
        .or_else(|| server_url.strip_prefix("http://").map(|rest| (false, rest)))?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let assumed = if secure { SECURE } else { PLAIN };
    let (host, port) = if authority.starts_with('[') {
        let (host, after) = authority.split_at(authority.find(']')? + 1);
        let port = if after.is_empty() {
            assumed
        } else {
            after.strip_prefix(':')?.parse().ok()?
        };
        (host, port)
    } else {
        match authority.split_once(':') {
            Some((host, port)) => (host, port.parse().ok()?),
            None => (authority, assumed),
        }
    };
    let path = path.trim_end_matches('/');
    let base = if path.is_empty() {
        String::new()
    } else {
        format!("/{path}")
    };
    an_address(host, port, &base).then_some(Reached {
        secure,
        host,
        port,
        base,
    })
}

fn an_address(host: &str, port: u16, base: &str) -> bool {
    port != 0 && a_host(host) && a_base(base)
}

fn a_host(host: &str) -> bool {
    if let Some(inside) = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        return inside.contains(':')
            && inside
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b':' || byte == b'.');
    }
    (1..=LONGEST_HOST).contains(&host.len())
        && !host.starts_with(['-', '.'])
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
}

fn a_base(base: &str) -> bool {
    base.is_empty()
        || (base.starts_with('/')
            && !base.ends_with('/')
            && base
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || BASE_MARKS.contains(&byte))
            && base
                .split('/')
                .skip(1)
                .all(|segment| !matches!(segment, "" | "." | "..")))
}

fn telling(enabled: bool, bits: u32) -> Telling {
    let named = OCCASION_BITS.iter().fold(0, |field, (_, bit)| field | bit);
    Telling {
        enabled,
        occasions: OCCASION_BITS
            .iter()
            .filter(|(_, bit)| bits & bit != 0)
            .map(|&(occasion, _)| occasion)
            .collect(),
        others: bits & !named != 0,
    }
}

fn bits(occasions: &BTreeSet<Occasion>) -> u32 {
    OCCASION_BITS
        .iter()
        .filter(|(occasion, _)| occasions.contains(occasion))
        .fold(0, |field, (_, bit)| field | bit)
}

/// A client for one Seerr, presenting the key `held` supplies on every call.
pub struct Seerr {
    endpoint: Endpoint,
    held: Arc<dyn Holding>,
}

impl Seerr {
    /// The Seerr reached at `base` over `http`, asked with the key `held` supplies.
    #[must_use]
    pub fn new(http: Arc<dyn Http>, base: impl Into<String>, held: Arc<dyn Holding>) -> Self {
        Self {
            endpoint: Endpoint::new(http, base, SERVICE),
            held,
        }
    }

    /// Whether Seerr answers its status, which it serves without a key.
    pub async fn answers(&self) -> bool {
        let request = Request {
            method: Method::Get,
            url: self.endpoint.url(&format!("{API}{STATUS}")),
            headers: Vec::new(),
            body: None,
            pinned: None,
        };
        self.endpoint
            .send(&request)
            .await
            .is_ok_and(|response| response.is_success())
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<String>,
    ) -> Result<Request, Failure> {
        let key = self.held.key().map_err(|why| {
            self.endpoint
                .refused(&format!("this adapter holds no key for Seerr: {why}"))
        })?;
        let mut headers = vec![(KEY_HEADER.to_owned(), key)];
        if body.is_some() {
            headers.push((CONTENT_TYPE.to_owned(), JSON.to_owned()));
        }
        Ok(Request {
            method,
            url: self.endpoint.url(&format!("{API}{path}")),
            headers,
            body,
            pinned: None,
        })
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<String>,
    ) -> Result<Response, Failure> {
        self.endpoint.send(&self.request(method, path, body)?).await
    }

    async fn send_as(
        &self,
        method: Method,
        path: &str,
        body: Option<String>,
        member: u64,
    ) -> Result<Response, Failure> {
        let mut request = self.request(method, path, body)?;
        request
            .headers
            .push((ACTING_AS.to_owned(), member.to_string()));
        self.endpoint.send(&request).await
    }

    async fn signed_in_naming(
        &self,
        username: &str,
        password: &str,
        server_url: &str,
    ) -> Result<(), Failure> {
        let Some(jellyfin) = taken_apart(server_url) else {
            return Err(self.endpoint.refused(NOT_AN_ADDRESS));
        };
        let body = serde_json::json!({
            "username": username,
            "password": password,
            "hostname": jellyfin.host,
            "port": jellyfin.port,
            "useSsl": jellyfin.secure,
            "urlBase": jellyfin.base,
            "email": OWNER_EMAIL,
            "serverType": JELLYFIN_SERVER_TYPE,
        })
        .to_string();
        let signed_in = self
            .send(Method::Post, "/auth/jellyfin", Some(body))
            .await?;
        self.endpoint.expect_success(&signed_in)
    }
}

#[derive(Deserialize)]
struct PublicSettings {
    #[serde(default)]
    initialized: bool,
}

#[derive(Deserialize)]
struct WebPush {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    types: u32,
}

#[async_trait]
impl Requests for Seerr {
    async fn initialized(&self) -> Result<bool, Failure> {
        let response = self.send(Method::Get, "/settings/public", None).await?;
        let settings: PublicSettings = self
            .endpoint
            .decode(&response, "the public settings could not be read")?;
        Ok(settings.initialized)
    }

    async fn answers(&self) -> Result<(), Failure> {
        let response = self.send(Method::Get, "/auth/me", None).await?;
        self.endpoint.expect_success(&response)
    }

    async fn configure_identity(&self, source: &IdentitySource) -> Result<(), Failure> {
        if source.protocol.0 != JELLYFIN {
            return Err(self.endpoint.unsupported(&format!(
                "it signs a household in through no identity source spoken to as {}",
                source.protocol.0
            )));
        }
        let Credential::UserPass { username, password } = &source.credential else {
            return Err(self
                .endpoint
                .unsupported("it signs a household in by an administrator's name and password"));
        };
        self.signed_in_naming(username, password, &source.at)
            .await?;
        let finished = self
            .send(Method::Post, "/settings/initialize", None)
            .await?;
        self.endpoint.expect_success(&finished)
    }

    async fn requests(&self) -> Result<Vec<HouseholdRequest>, Failure> {
        let mut requests = Vec::new();
        let mut skip = 0;
        loop {
            let path =
                format!("/request?take={REQUEST_PAGE}&skip={skip}&sort=added&sortDirection=desc");
            let response = self.send(Method::Get, &path, None).await?;
            let page: RequestPage = self
                .endpoint
                .decode(&response, "the household's requests could not be read")?;
            let on_this_page = page.results.len();
            requests.extend(page.results.into_iter().map(RequestRecord::into_request));
            if on_this_page < REQUEST_PAGE || requests.len() >= page.page_info.results {
                break;
            }
            skip += REQUEST_PAGE;
        }
        Ok(requests)
    }

    async fn fulfilment_targets(&self) -> Result<Vec<RegisteredTarget>, Failure> {
        targets::fulfilment_targets(self).await
    }

    async fn add_fulfilment_target(&self, target: &FulfilmentTarget) -> Result<(), Failure> {
        targets::add_fulfilment_target(self, target).await
    }

    async fn move_fulfilment_target(
        &self,
        held: &RegisteredTarget,
        at: &lemonfiber_ports::service::Endpoint,
        key: &str,
    ) -> Result<(), Failure> {
        targets::move_fulfilment_target(self, held, at, key).await
    }

    async fn test_fulfilment_target(
        &self,
        kind: Kind,
        at: &lemonfiber_ports::service::Endpoint,
        key: &str,
    ) -> Result<(), Failure> {
        targets::test_fulfilment_target(self, kind, at, key).await
    }

    async fn media_server_link(&self) -> Result<MediaServerLink, Failure> {
        linking::media_server_link(self).await
    }

    async fn link_media_server(
        &self,
        at: &lemonfiber_ports::service::Endpoint,
        key: &str,
    ) -> Result<(), Failure> {
        linking::link_media_server(self, at, key).await
    }

    async fn link_members(&self, members: &[String]) -> Result<(), Failure> {
        if members.is_empty() {
            return Ok(());
        }
        let body = serde_json::json!({ "jellyfinUserIds": members }).to_string();
        let written = self.send(Method::Post, LINK_MEMBERS, Some(body)).await?;
        self.endpoint.expect_success(&written)
    }

    async fn member_for(&self, media_server_id: &str) -> Result<Option<String>, Failure> {
        Ok(self
            .member(
                media_server_id,
                "the account Seerr holds for them could not be read",
            )
            .await?
            .map(|held| held.id.to_string()))
    }

    async fn requesting(&self, media_server_id: &str) -> Result<Option<Requesting>, Failure> {
        Ok(self
            .member(
                media_server_id,
                "what this member may ask for could not be read",
            )
            .await?
            .map(|held| Requesting {
                id: held.id.to_string(),
                approves_own: approves_own(held.permissions),
            }))
    }

    async fn approval_first(&self, id: &str) -> Result<(), Failure> {
        lemonfiber_ports::service::Approving::approves_own(self, id, false).await
    }

    async fn remove_member(&self, id: &str) -> Result<(), Failure> {
        let gone = self
            .send(Method::Delete, &self.member_path(id)?, None)
            .await?;
        self.endpoint.expect_success(&gone)
    }

    async fn telling(&self) -> Result<Telling, Failure> {
        let response = self.send(Method::Get, WEBPUSH, None).await?;
        let held: WebPush = self
            .endpoint
            .decode(&response, "what the household is told could not be read")?;
        Ok(telling(held.enabled, held.types))
    }

    async fn tell(&self, telling: &Telling) -> Result<(), Failure> {
        let body = serde_json::json!({
            "enabled": telling.enabled,
            "types": bits(&telling.occasions),
        })
        .to_string();
        let written = self.send(Method::Post, WEBPUSH, Some(body)).await?;
        self.endpoint.expect_success(&written)
    }
}

impl Seerr {
    fn held_to_an_address(&self, at: &lemonfiber_ports::service::Endpoint) -> Result<(), Failure> {
        if an_address(&at.host, at.port, &at.base) {
            Ok(())
        } else {
            Err(self.endpoint.refused(NOT_AN_ADDRESS))
        }
    }

    fn member_path(&self, id: &str) -> Result<String, Failure> {
        if (1..=LONGEST_NUMBER).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_digit())
        {
            Ok(format!("{MEMBERS}/{id}"))
        } else {
            Err(self
                .endpoint
                .refused("that is not an account Seerr numbers"))
        }
    }

    async fn member(
        &self,
        media_server_id: &str,
        whenever: &str,
    ) -> Result<Option<MemberResource>, Failure> {
        if !an_identity(media_server_id) {
            return Err(self
                .endpoint
                .refused("that is not an account the identity source names"));
        }
        let response = self
            .send(
                Method::Get,
                &format!("{MEMBERS}/{JELLYFIN}/{media_server_id}"),
                None,
            )
            .await?;
        if response.status == NOT_FOUND {
            return Ok(None);
        }
        self.endpoint.decode(&response, whenever).map(Some)
    }
}

fn an_identity(id: &str) -> bool {
    let digits = id.bytes().filter(|byte| *byte != b'-').count();
    digits == IDENTITY_DIGITS
        && id.len() <= IDENTITY_DIGITS + IDENTITY_DASHES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

#[cfg(test)]
mod tests;
