//! Thin HTTP client over the Iyke server. All requests go to
//! `http://127.0.0.1:<port>` with a `Authorization: Bearer <token>`
//! header. Errors are normalized into a single `ApiError` so the main
//! command dispatcher can render them uniformly.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::control::ControlFile;

pub struct Client {
    base: String,
    token: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(cf: &ControlFile) -> Self {
        Self {
            base: format!("http://127.0.0.1:{}", cf.port),
            token: cf.token.clone(),
            agent: ureq::AgentBuilder::new()
                .timeout_connect(std::time::Duration::from_secs(2))
                .timeout(std::time::Duration::from_secs(5))
                .build(),
        }
    }

    pub fn get_state(&self) -> Result<Value> {
        let resp = self
            .agent
            .get(&format!("{}/iyke/state", self.base))
            .set("Authorization", &format!("Bearer {}", self.token))
            .call();
        normalize_response(resp, "/iyke/state")
    }

    pub fn post(&self, path: &str, body: Value) -> Result<Value> {
        let resp = self
            .agent
            .post(&format!("{}{}", self.base, path))
            .set("Authorization", &format!("Bearer {}", self.token))
            .send_json(body);
        normalize_response(resp, path)
    }
}

fn normalize_response(
    resp: Result<ureq::Response, ureq::Error>,
    path: &str,
) -> Result<Value> {
    match resp {
        Ok(r) => {
            // Empty 200 from write endpoints — we still want valid JSON
            // back to callers so they can pretty-print or pipe into jq.
            let body = r.into_string().unwrap_or_default();
            if body.is_empty() {
                return Ok(serde_json::json!({"ok": true}));
            }
            serde_json::from_str(&body)
                .with_context(|| format!("parse response from {path}: {body}"))
        }
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            Err(anyhow!("{path} returned HTTP {code}: {body}"))
        }
        Err(ureq::Error::Transport(t)) => Err(anyhow!(
            "could not reach iyke server at {path}: {t}. Is the PA desktop app running?"
        )),
    }
}
