use std::sync::Arc;

use lemonfiber_ports::http::{Http, Request, Response};
use lemonfiber_ports::service::Failure;

const DETAIL_LIMIT: usize = 200;

const MARKER: &str = "...";

/// A value written as one query parameter's value: every byte but the unreserved ones
/// percent-encoded.
pub fn query_encoded(value: &str) -> String {
    value.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.extend(format!("%{byte:02X}").chars());
        }
        out
    })
}

/// A service reached over the HTTP port, and what to call it when something goes wrong.
pub struct Endpoint {
    http: Arc<dyn Http>,
    base: String,
    service: String,
}

impl Endpoint {
    /// An endpoint for `service`, reached at `base` over `http`.
    pub fn new(http: Arc<dyn Http>, base: impl Into<String>, service: impl Into<String>) -> Self {
        Self {
            http,
            base: base.into(),
            service: service.into(),
        }
    }

    /// A URL for `path` under the base, the base's trailing slash dropped.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base.trim_end_matches('/'))
    }

    /// Send a request, a transport failure read as the service not answering.
    pub async fn send(&self, request: &Request) -> Result<Response, Failure> {
        self.http
            .send(request)
            .await
            .map_err(|_| self.unavailable())
    }

    fn unavailable(&self) -> Failure {
        Failure::Unavailable {
            service: self.service.clone(),
        }
    }

    /// A non-success response: a refused credential, or an answer that cannot be used.
    pub fn refusal(&self, response: &Response) -> Failure {
        match response.status {
            401 | 403 => self.unauthorised(),
            _ => self.refused(&describe(response)),
        }
    }

    /// A rejected credential, blamed on the service.
    pub fn unauthorised(&self) -> Failure {
        Failure::Unauthorised {
            service: self.service.clone(),
        }
    }

    /// An answer that arrived and could not be used, for `detail`.
    pub fn refused(&self, detail: &str) -> Failure {
        Failure::Refused {
            service: self.service.clone(),
            detail: detail.to_owned(),
        }
    }

    /// A question this service cannot be asked, for `detail`.
    pub fn unsupported(&self, detail: &str) -> Failure {
        Failure::Unsupported {
            service: self.service.clone(),
            detail: detail.to_owned(),
        }
    }

    /// Nothing where the response succeeded, the refusal its status amounts to otherwise.
    pub fn expect_success(&self, response: &Response) -> Result<(), Failure> {
        if response.is_success() {
            Ok(())
        } else {
            Err(self.refusal(response))
        }
    }

    /// A JSON body read as `T`, a body that does not parse named by `what`.
    pub fn decode<T: serde::de::DeserializeOwned>(
        &self,
        response: &Response,
        what: &str,
    ) -> Result<T, Failure> {
        if !response.is_success() {
            return Err(self.refusal(response));
        }
        serde_json::from_str(&response.body).map_err(|err| self.refused(&format!("{what}: {err}")))
    }
}

/// A non-success response as a refusal's detail: the service's own words, withheld and
/// then shortened, or its status alone where it said none.
pub fn describe(response: &Response) -> String {
    let body = lemonfiber_error::withheld::withheld_text(response.body.trim());
    let body = body.trim();
    if body.is_empty() {
        format!("HTTP {}", response.status)
    } else {
        format!("HTTP {}: {}", response.status, fitted(body, DETAIL_LIMIT))
    }
}

fn fitted(text: &str, width: usize) -> String {
    let counted = text.chars().count();
    if counted <= width {
        return text.to_owned();
    }
    let keep = width.saturating_sub(MARKER.len());
    let tail = keep / 2;
    let head = keep - tail;
    let front: String = text.chars().take(head).collect();
    let back: String = text.chars().skip(counted - tail).collect();
    format!("{front}{MARKER}{back}")
}

#[cfg(test)]
mod tests;
