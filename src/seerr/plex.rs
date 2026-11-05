//! Seerr signed in through Plex (ARCH-R379): its owner made by signing in with a Plex token,
//! Plex reached at the request gate's Plex route, and members linked and found by their Plex
//! ids.

use std::collections::BTreeSet;

use lemonfiber_ports::http::Method;
use lemonfiber_ports::service::{Endpoint, Failure, MediaServerLink};
use serde::Deserialize;

use super::members::{MemberResource, MEMBERS};
use super::{unsupported, Reached, Seerr, HELD_ELSEWHERE, NO_MEDIA_SERVER};

/// The identity source Seerr signs a household in through, as the stack names Plex's API.
pub const PLEX: &str = "plex";

/// How Seerr numbers Plex among the media servers it serves.
pub(super) const PLEX_SERVER_TYPE: u8 = 1;

const SIGN_IN: &str = "/auth/plex";

const HELD_PLEX: &str = "/settings/plex";

const IMPORT: &str = "/user/import-from-plex";

/// How many accounts are read at a time.
const MEMBER_PAGE: usize = 100;

/// The longest Plex id, in digits.
const LONGEST_ID: usize = 19;

/// Why an empty key signs Seerr in as nobody.
pub(super) const EMPTY_KEY: &str =
    "an empty key signs nobody in: Seerr's owner signs in through Plex with a token, or with none where the operator signs in on Seerr's own page";

/// Why Seerr with no owner is not set up where no key is carried.
pub(super) const OWNER_SIGNS_IN: &str = "Seerr has no owner yet: the operator signs in through Plex on Seerr's own page, which makes them its owner";

const NOT_A_PLEX_ADDRESS: &str =
    "Seerr reaches Plex at a host and a port alone, with no path beneath them";

const NO_PLEX_KEY: &str =
    "Seerr reaches Plex with its owner's token, so it is handed no key for Plex";

const NONE_LINKED: &str = "Seerr holds none of these members by their Plex ids: plex.tv lists none of them on its owner, so lemonfiber asks as its owner on their behalf";

const NOT_A_PLEX_ID: &str = "that is not an account plex.tv numbers";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeldPlex {
    #[serde(default)]
    ip: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    use_ssl: Option<bool>,
}

impl HeldPlex {
    fn is(&self, route: &Reached<'_>) -> bool {
        self.ip == route.host
            && self.port == route.port
            && self.use_ssl.unwrap_or(false) == route.secure
    }
}

#[derive(Deserialize)]
struct MemberPage {
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
    #[serde(default)]
    results: Vec<MemberResource>,
}

#[derive(Deserialize)]
struct PageInfo {
    #[serde(default)]
    results: usize,
}

/// Whether `id` is spelled as plex.tv numbers an account: digits alone.
pub(super) fn a_plex_id(id: &str) -> bool {
    (1..=LONGEST_ID).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_digit())
}

/// Sign Seerr in with the Plex token `token`, which makes its owner on a fresh Seerr.
pub(super) async fn signed_in(seerr: &Seerr, token: &str) -> Result<(), Failure> {
    let body = serde_json::json!({ "authToken": token }).to_string();
    let signed_in = seerr.send(Method::Post, SIGN_IN, Some(body)).await?;
    seerr.endpoint.expect_success(&signed_in)
}

/// Make Seerr's owner through Plex with `token` where it has none, or wait for the operator's
/// own sign-in where none is carried, and point it at Plex at `route`.
pub(super) async fn configured(
    seerr: &Seerr,
    token: Option<&str>,
    route: &Reached<'_>,
    held: Option<u8>,
) -> Result<(), Failure> {
    if !route.base.is_empty() {
        return Err(seerr.endpoint.refused(NOT_A_PLEX_ADDRESS));
    }
    match (held, token) {
        (Some(NO_MEDIA_SERVER), Some(token)) => signed_in(seerr, token).await?,
        (Some(NO_MEDIA_SERVER), None) => return Err(seerr.endpoint.refused(OWNER_SIGNS_IN)),
        (Some(PLEX_SERVER_TYPE), _) => {}
        _ => return Err(seerr.endpoint.refused(HELD_ELSEWHERE)),
    }
    let plex = held_plex(seerr).await?;
    if plex.is(route) {
        return Ok(());
    }
    if !plex.ip.is_empty() {
        return Err(seerr.endpoint.refused(HELD_ELSEWHERE));
    }
    written(seerr, route.host, route.port, route.secure).await
}

async fn held_plex(seerr: &Seerr) -> Result<HeldPlex, Failure> {
    let response = seerr.send(Method::Get, HELD_PLEX, None).await?;
    seerr
        .endpoint
        .decode(&response, "the Plex Seerr holds could not be read")
}

async fn written(seerr: &Seerr, host: &str, port: u16, secure: bool) -> Result<(), Failure> {
    let body = serde_json::json!({ "ip": host, "port": port, "useSsl": secure }).to_string();
    let written = seerr.send(Method::Post, HELD_PLEX, Some(body)).await?;
    seerr.endpoint.expect_success(&written)
}

/// Where Seerr reaches Plex, with no key: it reaches Plex with its owner's token.
pub(super) async fn link(seerr: &Seerr) -> Result<MediaServerLink, Failure> {
    let plex = held_plex(seerr).await?;
    Ok(MediaServerLink {
        at: Endpoint {
            host: plex.ip,
            port: plex.port,
            base: String::new(),
        },
        key: String::new(),
    })
}

/// Point Seerr at Plex at `at`, refusing a key or a path beneath it.
pub(super) async fn linked(seerr: &Seerr, at: &Endpoint, key: &str) -> Result<(), Failure> {
    if !key.is_empty() {
        return Err(seerr.endpoint.refused(NO_PLEX_KEY));
    }
    if !at.base.is_empty() {
        return Err(seerr.endpoint.refused(NOT_A_PLEX_ADDRESS));
    }
    written(seerr, &at.host, at.port, false).await
}

/// Every account Seerr holds, read a page at a time.
async fn every_member(seerr: &Seerr, whenever: &str) -> Result<Vec<MemberResource>, Failure> {
    let mut held = Vec::new();
    let mut skip = 0;
    loop {
        let path = format!("{MEMBERS}?take={MEMBER_PAGE}&skip={skip}&sort=created");
        let response = seerr.send(Method::Get, &path, None).await?;
        let page: MemberPage = seerr.endpoint.decode(&response, whenever)?;
        let on_this_page = page.results.len();
        held.extend(page.results);
        skip += MEMBER_PAGE;
        if on_this_page < MEMBER_PAGE || skip >= page.page_info.results {
            return Ok(held);
        }
    }
}

/// The account Seerr holds for the Plex account `id`, or none.
pub(super) async fn member(
    seerr: &Seerr,
    id: &str,
    whenever: &str,
) -> Result<Option<MemberResource>, Failure> {
    let wanted: i64 = id
        .parse()
        .map_err(|_| seerr.endpoint.refused(NOT_A_PLEX_ID))?;
    Ok(every_member(seerr, whenever)
        .await?
        .into_iter()
        .find(|held| held.plex_id == Some(wanted)))
}

/// Import the plex.tv accounts `members` from those plex.tv lists on Seerr's owner, then hold
/// that Seerr holds at least one of them; a member it holds no account for is asked for by
/// lemonfiber as its owner.
pub(super) async fn members_linked(seerr: &Seerr, members: &[String]) -> Result<(), Failure> {
    let wanted: BTreeSet<i64> = members
        .iter()
        .map(|id| {
            a_plex_id(id)
                .then(|| id.parse().ok())
                .flatten()
                .ok_or_else(|| seerr.endpoint.refused(NOT_A_PLEX_ID))
        })
        .collect::<Result<_, _>>()?;
    let body = serde_json::json!({ "plexIds": members }).to_string();
    let imported = seerr.send(Method::Post, IMPORT, Some(body)).await?;
    let held = every_member(seerr, "the accounts Seerr holds could not be read").await?;
    let linked = held
        .iter()
        .filter_map(|one| one.plex_id)
        .filter(|id| wanted.contains(id))
        .count();
    if linked == 0 {
        return Err(unsupported(NONE_LINKED));
    }
    if linked < wanted.len() {
        return seerr.endpoint.expect_success(&imported);
    }
    Ok(())
}
