use async_trait::async_trait;
use serde::Deserialize;

use super::Seerr;
use lemonfiber_ports::http::Method;
use lemonfiber_ports::service::{Failure, Noticing};

const SLIDERS: &str = "/settings/discover";

const ADD: &str = "/settings/discover/add";

const OURS: &str = "lemonfiber";

const SEARCHING: u8 = 17;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    #[serde(default)]
    id: i64,
    #[serde(default, rename = "type")]
    kind: u8,
    #[serde(default)]
    is_built_in: bool,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    data: Option<String>,
}

impl Row {
    fn ours(&self) -> bool {
        !self.is_built_in && self.data.as_deref() == Some(OURS)
    }

    fn sentence(&self) -> String {
        self.title.clone().unwrap_or_default()
    }

    fn written(&self, order: usize, enabled: bool) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "type": self.kind,
            "isBuiltIn": self.is_built_in,
            "enabled": enabled,
            "order": order,
            "title": self.title,
            "data": self.data,
        })
    }
}

impl Seerr {
    async fn rows(&self) -> Result<Vec<Row>, Failure> {
        let response = self.send(Method::Get, SLIDERS, None).await?;
        self.endpoint.decode(
            &response,
            "what Seerr is showing the household could not be read",
        )
    }

    async fn shown(&self) -> Result<(), Failure> {
        let held = self.rows().await?;
        let (ours, theirs): (Vec<&Row>, Vec<&Row>) = held.iter().partition(|row| row.ours());
        let body: Vec<serde_json::Value> = ours
            .iter()
            .map(|row| (*row, true))
            .chain(theirs.iter().map(|row| (*row, row.enabled)))
            .enumerate()
            .map(|(order, (row, enabled))| row.written(order, enabled))
            .collect();
        let written = self
            .send(
                Method::Post,
                SLIDERS,
                Some(serde_json::to_string(&body).unwrap_or_default()),
            )
            .await?;
        self.endpoint.expect_success(&written)
    }
}

#[async_trait]
impl Noticing for Seerr {
    async fn set_notices(&self, notices: &[String]) -> Result<(), Failure> {
        set_notices(self, notices).await
    }
}

async fn set_notices(seerr: &Seerr, notices: &[String]) -> Result<(), Failure> {
    let held = seerr.rows().await?;
    let showing: Vec<String> = held
        .iter()
        .filter(|row| row.ours())
        .map(Row::sentence)
        .collect();
    if showing == notices {
        return Ok(());
    }
    for row in held.iter().filter(|row| row.ours()) {
        let path = format!("{SLIDERS}/{}", row.id);
        let removed = seerr.send(Method::Delete, &path, None).await?;
        seerr.endpoint.expect_success(&removed)?;
    }
    for notice in notices {
        let body = serde_json::json!({
            "type": SEARCHING,
            "title": notice,
            "data": OURS,
        })
        .to_string();
        let added = seerr.send(Method::Post, ADD, Some(body)).await?;
        seerr.endpoint.expect_success(&added)?;
    }
    seerr.shown().await
}

#[cfg(test)]
mod tests;
