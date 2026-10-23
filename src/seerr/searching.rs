use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::records::{kind_of, media_status, media_type, request_status};
use super::{Seerr, NOT_FOUND};
use lemonfiber_ports::http::Method;
use lemonfiber_ports::media::Kind;
use lemonfiber_ports::service::{
    Asked, Detail, Failure, Found, MediaStatus, Page, RequestStatus, Searching, Season, Wish,
};

const RELEASED: &str = "Released";

const UNRELEASED_SERIES: [&str; 3] = ["Planned", "In Production", "Pilot"];

const SPECIALS: u32 = 0;

const FORBIDDEN: u16 = 403;

const MAY_NOT_ASK: &str = "the member may not ask for this";

const POSTERS: &str = "https://image.tmdb.org/t/p/w342";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchPage {
    #[serde(default)]
    page: u32,
    #[serde(default)]
    total_pages: u32,
    #[serde(default)]
    results: Vec<Hit>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hit {
    id: i64,
    media_type: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    first_air_date: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    media_info: Option<MediaInfo>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaInfo {
    #[serde(default)]
    status: u8,
    #[serde(default)]
    seasons: Vec<SeasonInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeasonInfo {
    season_number: u32,
    #[serde(default)]
    status: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Full {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    first_air_date: Option<String>,
    #[serde(default)]
    releases: Option<Releases>,
    #[serde(default)]
    content_ratings: Option<Ratings>,
    #[serde(default)]
    seasons: Vec<SeasonNumber>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    media_info: Option<MediaInfo>,
}

#[derive(Deserialize)]
struct Releases {
    #[serde(default)]
    results: Vec<RegionReleases>,
}

#[derive(Deserialize)]
struct RegionReleases {
    iso_3166_1: String,
    #[serde(default)]
    release_dates: Vec<ReleaseDate>,
}

#[derive(Deserialize)]
struct ReleaseDate {
    #[serde(default)]
    certification: String,
}

#[derive(Deserialize)]
struct Ratings {
    #[serde(default)]
    results: Vec<Rating>,
}

#[derive(Deserialize)]
struct Rating {
    iso_3166_1: String,
    #[serde(default)]
    rating: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeasonNumber {
    season_number: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Filing {
    media_type: &'static str,
    media_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    seasons: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Filed {
    id: i64,
    #[serde(default)]
    status: u8,
}

fn number(id: &str) -> Option<u64> {
    id.parse().ok()
}

fn year_of(date: Option<&str>) -> Option<u16> {
    date?.get(..4)?.parse().ok()
}

fn standing(info: Option<&MediaInfo>) -> MediaStatus {
    info.and_then(|info| media_status(info.status))
        .unwrap_or(MediaStatus::Unknown)
}

impl Hit {
    fn found(self, kinds: &[Kind]) -> Option<Found> {
        let kind = kind_of(&self.media_type).filter(|kind| kinds.contains(kind))?;
        Some(Found {
            id: self.id.to_string(),
            kind,
            title: self.title.or(self.name)?,
            year: year_of(self.release_date.or(self.first_air_date).as_deref()),
            status: standing(self.media_info.as_ref()),
            poster: published(self.poster_path),
        })
    }
}

impl Full {
    fn detail(self, kind: Kind, region: &str) -> Detail {
        let certification = match kind {
            Kind::Movies => self.releases.and_then(|releases| {
                releases
                    .results
                    .into_iter()
                    .find(|one| one.iso_3166_1.eq_ignore_ascii_case(region))?
                    .release_dates
                    .into_iter()
                    .map(|date| date.certification)
                    .find(|certification| !certification.is_empty())
            }),
            Kind::Tv => self.content_ratings.and_then(|ratings| {
                ratings
                    .results
                    .into_iter()
                    .find(|one| one.iso_3166_1.eq_ignore_ascii_case(region))
                    .map(|one| one.rating)
                    .filter(|rating| !rating.is_empty())
            }),
        };
        let year = year_of(
            self.release_date
                .as_deref()
                .or(self.first_air_date.as_deref()),
        );
        let status = self.status.as_deref().unwrap_or_default();
        let released = match kind {
            Kind::Movies => status == RELEASED,
            Kind::Tv => self.first_air_date.is_some() && !UNRELEASED_SERIES.contains(&status),
        };
        let whole = standing(self.media_info.as_ref());
        let held = self.media_info.unwrap_or_default().seasons;
        let seasons = self
            .seasons
            .into_iter()
            .map(|season| season.season_number)
            .filter(|number| *number != SPECIALS)
            .map(|number| Season {
                number,
                status: held
                    .iter()
                    .find(|one| one.season_number == number)
                    .and_then(|one| media_status(one.status))
                    .unwrap_or(MediaStatus::Unknown),
            })
            .collect();
        Detail {
            title: self.title.or(self.name).filter(|title| !title.is_empty()),
            year,
            status: whole,
            overview: self.overview.filter(|overview| !overview.is_empty()),
            certification,
            released,
            seasons,
            poster: published(self.poster_path),
        }
    }
}

fn published(path: Option<String>) -> Option<String> {
    path.filter(|path| path.starts_with('/'))
        .map(|path| format!("{POSTERS}{path}"))
}

#[async_trait]
impl Searching for Seerr {
    async fn search(&self, term: &str, kinds: &[Kind], page: u32) -> Result<Page, Failure> {
        let path = format!("/search?query={}&page={page}", query_encoded(term));
        let response = self.send(Method::Get, &path, None).await?;
        let read: SearchPage = self
            .endpoint
            .decode(&response, "the search could not be read")?;
        Ok(Page {
            next: (read.page < read.total_pages).then_some(read.page + 1),
            titles: read
                .results
                .into_iter()
                .filter_map(|one| one.found(kinds))
                .collect(),
        })
    }

    async fn detail(&self, kind: Kind, id: &str, region: &str) -> Result<Option<Detail>, Failure> {
        detail(self, kind, id, region).await
    }

    async fn ask(&self, member: &str, wish: &Wish) -> Result<Asked, Failure> {
        ask(self, member, wish).await
    }
}

async fn detail(
    seerr: &Seerr,
    kind: Kind,
    id: &str,
    region: &str,
) -> Result<Option<Detail>, Failure> {
    let Some(id) = number(id) else {
        return Ok(None);
    };
    let path = format!("/{}/{id}", media_type(kind));
    let response = seerr.send(Method::Get, &path, None).await?;
    if response.status == NOT_FOUND {
        return Ok(None);
    }
    let full: Full = seerr
        .endpoint
        .decode(&response, "the title could not be read")?;
    Ok(Some(full.detail(kind, region)))
}

async fn ask(seerr: &Seerr, member: &str, wish: &Wish) -> Result<Asked, Failure> {
    let (Some(media), Some(user)) = (number(&wish.id), number(member)) else {
        return Err(seerr
            .endpoint
            .refused("the title or the member is not one Seerr names"));
    };
    let filing = Filing {
        media_type: media_type(wish.kind),
        media_id: media,
        seasons: (wish.kind == Kind::Tv).then(|| {
            if wish.seasons.is_empty() {
                serde_json::json!("all")
            } else {
                serde_json::json!(wish.seasons)
            }
        }),
    };
    let body = serde_json::to_string(&filing).unwrap_or_default();
    let response = seerr
        .send_as(Method::Post, "/request", Some(body), user)
        .await?;
    if response.status == FORBIDDEN {
        return Err(seerr.endpoint.refused(MAY_NOT_ASK));
    }
    let filed: Filed = seerr
        .endpoint
        .decode(&response, "the request could not be filed")?;
    Ok(Asked {
        request: filed.id,
        waiting: request_status(filed.status) == Some(RequestStatus::Pending),
    })
}

/// A value written as one query parameter's value: every byte but the unreserved ones
/// percent-encoded.
fn query_encoded(value: &str) -> String {
    value.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.extend(format!("%{byte:02X}").chars());
        }
        out
    })
}

#[cfg(test)]
mod tests;
