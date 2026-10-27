use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use ureq::http;

use crate::{Request, Variables, substitute};

/// Largest response body that is read into memory.
const BODY_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    /// Body as text; JSON bodies are pretty-printed.
    pub body: String,
    /// Whether `body` holds JSON.
    pub is_json: bool,
    pub size: usize,
    pub elapsed: Duration,
}

impl Response {
    pub fn headers_text(&self) -> String {
        self.headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Sends `request` after substituting `vars`. Blocks until the body is read.
pub fn execute(request: &Request, vars: &Variables) -> Result<Response> {
    let url = substitute(request.url.trim(), vars);
    anyhow::ensure!(!url.is_empty(), "URL is empty");
    let url = if url.contains("://") {
        url
    } else {
        format!("http://{url}")
    };

    let mut builder = http::Request::builder()
        .method(request.method.as_str())
        .uri(&url);
    for (name, value) in &request.headers {
        builder = builder.header(substitute(name, vars), substitute(value, vars));
    }

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();

    let started = Instant::now();
    let body = substitute(&request.body, vars);
    let mut response = if body.is_empty() {
        agent.run(builder.body(())?)
    } else {
        agent.run(builder.body(body.into_bytes())?)
    }
    .with_context(|| format!("{} {url}", request.method))?;

    let bytes = response
        .body_mut()
        .with_config()
        .limit(BODY_LIMIT)
        .read_to_vec()
        .context("reading response body")?;
    let elapsed = started.elapsed();

    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();

    let (body, is_json) = format_body(&bytes);
    Ok(Response {
        status: status.as_u16(),
        reason: status.canonical_reason().unwrap_or_default().to_string(),
        headers,
        body,
        is_json,
        size: bytes.len(),
        elapsed,
    })
}

fn format_body(bytes: &[u8]) -> (String, bool) {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes)
        && let Ok(pretty) = serde_json::to_string_pretty(&value)
    {
        return (pretty, true);
    }
    (String::from_utf8_lossy(bytes).into_owned(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Talks to httpbin.org: `cargo test -p reqo-core -- --ignored`.
    #[test]
    #[ignore = "needs network"]
    fn sends_post_with_variables() {
        let mut request = Request::new("post");
        request.method = "POST".into();
        request.url = "{{base}}/anything".into();
        request.headers.insert("X-Token".into(), "{{token}}".into());
        request.body = r#"{"n": 1}"#.into();
        let vars = Variables::from_iter([
            ("base".into(), "https://httpbin.org".into()),
            ("token".into(), "secret".into()),
        ]);

        let response = execute(&request, &vars).unwrap();
        assert_eq!(response.status, 200);
        assert!(response.is_json);
        assert!(response.body.contains("\"X-Token\": \"secret\""));
        assert!(response.body.contains("\"method\": \"POST\""));
    }

    #[test]
    #[ignore = "needs network"]
    fn error_status_is_a_response() {
        let mut request = Request::new("404");
        request.url = "https://httpbin.org/status/404".into();
        assert_eq!(execute(&request, &Variables::new()).unwrap().status, 404);
    }
}
