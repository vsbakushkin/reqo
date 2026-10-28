use std::path::Path;

use anyhow::{Context as _, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// HTTP methods offered in the UI, in display order.
pub const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

/// One saved request, stored as a `*.req.toml` file.
///
/// ```toml
/// name = "List users"
/// method = "GET"
/// url = "{{base}}/users"
///
/// [headers]
/// Accept = "application/json"
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub name: String,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub headers: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
}

fn default_method() -> String {
    "GET".into()
}

impl Request {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            method: default_method(),
            url: String::new(),
            headers: IndexMap::new(),
            body: String::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string(self)?;
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }

    /// Headers as `Name: value` lines, the form the UI edits them in.
    pub fn headers_text(&self) -> String {
        self.headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Headers of `inherited` (a collection's) that this request doesn't set
    /// itself. Names compare case-insensitively.
    pub fn inherited_headers<'a>(
        &self,
        inherited: &'a IndexMap<String, String>,
    ) -> impl Iterator<Item = (&'a String, &'a String)> {
        inherited.iter().filter(|(name, _)| {
            !self
                .headers
                .keys()
                .any(|own| own.eq_ignore_ascii_case(name))
        })
    }

    /// The request with the [`inherited_headers`](Self::inherited_headers)
    /// ahead of its own, as it is sent.
    pub fn with_inherited(&self, inherited: &IndexMap<String, String>) -> Request {
        let mut headers: IndexMap<String, String> = self
            .inherited_headers(inherited)
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        headers.extend(self.headers.clone());
        Request {
            headers,
            ..self.clone()
        }
    }

    /// Parses `Name: value` lines. Blank lines and lines starting with `#` are skipped.
    pub fn parse_headers(text: &str) -> IndexMap<String, String> {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.trim().to_string(), value.trim().to_string()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_round_trip() {
        let headers = Request::parse_headers("Accept: application/json\n\n# off\nX-Id:  42 ");
        assert_eq!(headers.len(), 2);
        assert_eq!(headers["X-Id"], "42");

        let mut request = Request::new("r");
        request.headers = headers;
        assert_eq!(request.headers_text(), "Accept: application/json\nX-Id: 42");
    }

    #[test]
    fn own_headers_win_over_inherited() {
        let inherited = Request::parse_headers("Authorization: Bearer {{token}}\nX-Team: core");
        let mut request = Request::new("r");
        request.headers = Request::parse_headers("x-team: web\nAccept: text/html");

        let kept: Vec<_> = request.inherited_headers(&inherited).collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].0, "Authorization");

        let sent = request.with_inherited(&inherited);
        assert_eq!(
            sent.headers_text(),
            "Authorization: Bearer {{token}}\nx-team: web\nAccept: text/html"
        );
    }

    #[test]
    fn toml_round_trip() {
        let mut request = Request::new("Create user");
        request.method = "POST".into();
        request.url = "{{base}}/users".into();
        request
            .headers
            .insert("Content-Type".into(), "application/json".into());
        request.body = "{\"name\": \"vasya\"}".into();

        let text = toml::to_string(&request).unwrap();
        assert_eq!(toml::from_str::<Request>(&text).unwrap(), request);
    }
}
