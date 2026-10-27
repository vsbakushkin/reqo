use std::path::Path;

use anyhow::{Context as _, Result};
use indexmap::IndexMap;

/// File in the collection root that holds the environments.
pub const ENV_FILE: &str = "reqo.env.toml";

pub type Variables = IndexMap<String, String>;

/// Named sets of variables, read from `reqo.env.toml`:
///
/// ```toml
/// [dev]
/// base = "http://localhost:8080"
///
/// [prod]
/// base = "https://api.example.com"
/// ```
#[derive(Debug, Clone, Default)]
pub struct Environments {
    envs: IndexMap<String, Variables>,
}

impl Environments {
    /// Loads environments from `dir`; a missing file means no environments.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(ENV_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let envs = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self { envs })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.envs.keys().map(String::as_str)
    }

    pub fn variables(&self, name: &str) -> Option<&Variables> {
        self.envs.get(name)
    }
}

/// Replaces `{{name}}` with its value. Unknown variables are left as they are.
pub fn substitute(text: &str, vars: &Variables) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let name = after[..end].trim();
                match vars.get(name) {
                    Some(value) => out.push_str(value),
                    None => out.push_str(&rest[start..start + 2 + end + 2]),
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_known_variables() {
        let vars = Variables::from_iter([("base".into(), "http://x".into())]);
        assert_eq!(
            substitute("{{base}}/a/{{ base }}", &vars),
            "http://x/a/http://x"
        );
        assert_eq!(
            substitute("{{missing}}/{{base", &vars),
            "{{missing}}/{{base"
        );
    }
}
