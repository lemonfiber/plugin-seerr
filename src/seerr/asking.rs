use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::members::{
    with_approval, with_asking, without_approval, without_asking, PermissionsResource, NOT_FOUND,
    PERMISSIONS,
};
use super::Seerr;
use lemonfiber_ports::http::Method;
use lemonfiber_ports::service::{Approving, Asking, Failure, Headroom, Holding, Left, Quota};

const MAIN: &str = "/settings/main";

const MEMBER_SETTINGS: &str = "settings/main";

const QUOTA: &str = "quota";

const APPROVE: &str = "approve";
const DECLINE: &str = "decline";

const AUTO_APPROVE: u64 = 128;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MainSettings {
    #[serde(default)]
    default_permissions: u64,
    #[serde(default)]
    default_quotas: DefaultQuotas,
}

#[derive(Default, Deserialize, Serialize)]
struct DefaultQuotas {
    #[serde(default)]
    movie: QuotaSetting,
    #[serde(default)]
    tv: QuotaSetting,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuotaSetting {
    #[serde(default)]
    quota_limit: Option<u32>,
    #[serde(default)]
    quota_days: Option<u32>,
}

impl QuotaSetting {
    fn of(quota: Option<Quota>) -> Self {
        match quota {
            Some(quota) => Self {
                quota_limit: Some(quota.requests),
                quota_days: Some(quota.days),
            },
            None => Self {
                quota_limit: Some(0),
                quota_days: None,
            },
        }
    }

    const fn quota(&self) -> Option<Quota> {
        match (self.quota_limit, self.quota_days) {
            (Some(requests), Some(days)) if requests > 0 => Some(Quota { requests, days }),
            _ => None,
        }
    }
}

#[derive(Default, Deserialize)]
struct Counts {
    #[serde(default)]
    movie: Counted,
    #[serde(default)]
    tv: Counted,
}

#[derive(Default, Deserialize)]
struct Counted {
    #[serde(default)]
    days: Option<u32>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    used: u32,
}

impl Counted {
    const fn left(&self) -> Left {
        Left {
            limit: match self.limit {
                Some(0) | None => None,
                Some(limit) => Some(limit),
            },
            used: self.used,
            days: self.days,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemberSettings {
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    discover_region: Option<String>,
    #[serde(default)]
    streaming_region: Option<String>,
    #[serde(default)]
    original_language: Option<String>,
    #[serde(default)]
    movie_quota_limit: Option<u32>,
    #[serde(default)]
    movie_quota_days: Option<u32>,
    #[serde(default)]
    tv_quota_limit: Option<u32>,
    #[serde(default)]
    tv_quota_days: Option<u32>,
    #[serde(default)]
    watchlist_sync_movies: Option<bool>,
    #[serde(default)]
    watchlist_sync_tv: Option<bool>,
}

impl MemberSettings {
    fn held_to(mut self, quota: Option<Quota>) -> Self {
        let (limit, days) = match quota {
            Some(quota) => (Some(quota.requests), Some(quota.days)),
            None => (Some(0), None),
        };
        self.movie_quota_limit = limit;
        self.movie_quota_days = days;
        self.tv_quota_limit = limit;
        self.tv_quota_days = days;
        self
    }
}

#[async_trait]
impl Approving for Seerr {
    async fn asking(&self) -> Result<Asking, Failure> {
        asking(self).await
    }

    async fn set_asking(&self, asking: &Asking) -> Result<(), Failure> {
        set_asking(self, asking).await
    }

    async fn left(&self, id: &str) -> Result<Headroom, Failure> {
        let path = format!("{}/{QUOTA}", self.member_path(id)?);
        let response = self.send(Method::Get, &path, None).await?;
        let held: Counts = self
            .endpoint
            .decode(&response, "what this member has left could not be read")?;
        Ok(Headroom {
            films: held.movie.left(),
            television: held.tv.left(),
        })
    }

    async fn set_quota(&self, id: &str, quota: Option<Quota>) -> Result<(), Failure> {
        let path = format!("{}/{MEMBER_SETTINGS}", self.member_path(id)?);
        let response = self.send(Method::Get, &path, None).await?;
        let held: MemberSettings = self
            .endpoint
            .decode(&response, "what this member is held to could not be read")?;
        let body = serde_json::to_string(&held.held_to(quota)).unwrap_or_default();
        let written = self.send(Method::Post, &path, Some(body)).await?;
        self.endpoint.expect_success(&written)
    }

    async fn approves_own(&self, id: &str, may: bool) -> Result<(), Failure> {
        approves_own(self, id, may).await
    }

    async fn decide(&self, request: i64, approve: bool) -> Result<(), Failure> {
        decide(self, request, approve).await
    }

    async fn hold_requests(&self, id: &str) -> Result<Holding, Failure> {
        hold_requests(self, id).await
    }

    async fn release_requests(&self, id: &str, holding: Holding) -> Result<(), Failure> {
        release_requests(self, id, holding).await
    }
}

impl Seerr {
    async fn asking_of(&self, id: &str) -> Result<Option<u64>, Failure> {
        let path = format!("{}/{PERMISSIONS}", self.member_path(id)?);
        let response = self.send(Method::Get, &path, None).await?;
        if response.status == NOT_FOUND {
            return Ok(None);
        }
        let held: PermissionsResource = self
            .endpoint
            .decode(&response, "what this member may ask for could not be read")?;
        Ok(Some(held.permissions))
    }

    async fn set_permissions(&self, id: &str, permissions: u64) -> Result<(), Failure> {
        let path = format!("{}/{PERMISSIONS}", self.member_path(id)?);
        let body = serde_json::json!({ "permissions": permissions }).to_string();
        let written = self.send(Method::Post, &path, Some(body)).await?;
        self.endpoint.expect_success(&written)
    }
}

async fn asking(seerr: &Seerr) -> Result<Asking, Failure> {
    let response = seerr.send(Method::Get, MAIN, None).await?;
    let held: MainSettings = seerr.endpoint.decode(
        &response,
        "what the household may ask for could not be read",
    )?;
    Ok(Asking {
        approves_own: held.default_permissions & AUTO_APPROVE != 0,
        quota: held
            .default_quotas
            .movie
            .quota()
            .or_else(|| held.default_quotas.tv.quota()),
    })
}

async fn set_asking(seerr: &Seerr, asking: &Asking) -> Result<(), Failure> {
    let response = seerr.send(Method::Get, MAIN, None).await?;
    let held: MainSettings = seerr.endpoint.decode(
        &response,
        "what the household may ask for could not be read",
    )?;
    let permissions = if asking.approves_own {
        with_approval(held.default_permissions)
    } else {
        without_approval(held.default_permissions)
    };
    let body = serde_json::json!({
        "defaultPermissions": permissions,
        "defaultQuotas": DefaultQuotas {
            movie: QuotaSetting::of(asking.quota),
            tv: QuotaSetting::of(asking.quota),
        },
    })
    .to_string();
    let written = seerr.send(Method::Post, MAIN, Some(body)).await?;
    seerr.endpoint.expect_success(&written)
}

async fn approves_own(seerr: &Seerr, id: &str, may: bool) -> Result<(), Failure> {
    let path = format!("{}/{PERMISSIONS}", seerr.member_path(id)?);
    let response = seerr.send(Method::Get, &path, None).await?;
    let held: PermissionsResource = seerr
        .endpoint
        .decode(&response, "what this member may ask for could not be read")?;
    let permissions = if may {
        with_approval(held.permissions)
    } else {
        without_approval(held.permissions)
    };
    seerr.set_permissions(id, permissions).await
}

async fn decide(seerr: &Seerr, request: i64, approve: bool) -> Result<(), Failure> {
    let said = if approve { APPROVE } else { DECLINE };
    let path = format!("/request/{request}/{said}");
    let ruled = seerr.send(Method::Post, &path, None).await?;
    seerr.endpoint.expect_success(&ruled)
}

async fn hold_requests(seerr: &Seerr, id: &str) -> Result<Holding, Failure> {
    let Some(held) = seerr.asking_of(id).await? else {
        return Ok(Holding::default());
    };
    let (left, taken) = without_asking(held);
    let holding = Holding { taken };
    if !holding.anything() {
        return Ok(holding);
    }
    seerr.set_permissions(id, left).await?;
    Ok(holding)
}

async fn release_requests(seerr: &Seerr, id: &str, holding: Holding) -> Result<(), Failure> {
    let Some(held) = seerr.asking_of(id).await? else {
        return Ok(());
    };
    seerr
        .set_permissions(id, with_asking(held, holding.taken))
        .await
}

#[cfg(test)]
mod tests;
