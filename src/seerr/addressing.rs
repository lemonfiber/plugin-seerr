use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Deserialize;

use super::members::MEMBERS;
use super::Seerr;
use lemonfiber_ports::http::Method;
use lemonfiber_ports::service::{Address, Addressing, Failure};

const REQUESTS: &str = "/request";

const NOTIFICATIONS: &str = "settings/notifications";

const DECLINED: u64 = 64;

const PUSHOVER: &str = "pushover";

const PUSHBULLET: &str = "pushbullet";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Asked {
    #[serde(default)]
    requested_by: Requester,
}

#[derive(Default, Deserialize)]
struct Requester {
    #[serde(default)]
    id: i64,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reachable {
    #[serde(default)]
    pushover_user_key: Option<String>,
    #[serde(default)]
    pushover_application_token: Option<String>,
    #[serde(default)]
    pushover_sound: Option<String>,
    #[serde(default)]
    pushbullet_access_token: Option<String>,
    #[serde(default)]
    notification_types: BTreeMap<String, serde_json::Value>,
}

impl Reachable {
    fn addresses(&self) -> Vec<Address> {
        [self.pushover(), self.pushbullet()]
            .into_iter()
            .flatten()
            .collect()
    }

    fn pushover(&self) -> Option<Address> {
        if !would_send(self.switch(PUSHOVER)) {
            return None;
        }
        Some(Address::Pushover {
            user: given(self.pushover_user_key.as_deref())?,
            application: given(self.pushover_application_token.as_deref())?,
            sound: given(self.pushover_sound.as_deref()),
        })
    }

    fn pushbullet(&self) -> Option<Address> {
        if !would_send(self.switch(PUSHBULLET)) {
            return None;
        }
        Some(Address::Pushbullet {
            token: given(self.pushbullet_access_token.as_deref())?,
        })
    }

    fn switch(&self, agent: &str) -> u64 {
        self.notification_types
            .get(agent)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default()
    }
}

const fn would_send(switch: u64) -> bool {
    switch & DECLINED != 0
}

fn given(field: Option<&str>) -> Option<String> {
    let written = field?.trim();
    (!written.is_empty()).then(|| written.to_owned())
}

#[async_trait]
impl Addressing for Seerr {
    async fn reachable(&self, request: i64) -> Result<Vec<Address>, Failure> {
        reachable(self, request).await
    }
}

async fn reachable(seerr: &Seerr, request: i64) -> Result<Vec<Address>, Failure> {
    let path = format!("{REQUESTS}/{request}");
    let filed = seerr.send(Method::Get, &path, None).await?;
    let asked: Asked = seerr
        .endpoint
        .decode(&filed, "who asked for this request could not be read")?;
    let path = format!("{MEMBERS}/{}/{NOTIFICATIONS}", asked.requested_by.id);
    let settings = seerr.send(Method::Get, &path, None).await?;
    let held: Reachable = seerr
        .endpoint
        .decode(&settings, "where this member is reached could not be read")?;
    Ok(held.addresses())
}

#[cfg(test)]
mod tests;
