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

    /// Writes the environments to `dir`. Comments in an existing file are not kept.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = dir.join(ENV_FILE);
        let text = toml::to_string(&self.envs)?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    }

    pub fn len(&self) -> usize {
        self.envs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.envs.is_empty()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.envs.keys().map(String::as_str)
    }

    pub fn variables(&self, name: &str) -> Option<&Variables> {
        self.envs.get(name)
    }

    /// Adds an empty environment named `base`, or `base-N` when taken, and returns its name.
    pub fn add(&mut self, base: &str) -> String {
        let name = (1..)
            .map(|n| match n {
                1 => base.to_string(),
                n => format!("{base}-{n}"),
            })
            .find(|name| !self.envs.contains_key(name))
            .expect("unbounded range");
        self.envs.insert(name.clone(), Variables::new());
        name
    }

    /// Replaces the variables of an existing environment.
    pub fn set_variables(&mut self, name: &str, vars: Variables) {
        if let Some(existing) = self.envs.get_mut(name) {
            *existing = vars;
        }
    }

    /// Renames an environment in place. Fails when `new` is empty or taken.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<()> {
        let new = new.trim();
        if new.is_empty() {
            anyhow::bail!("the name is empty");
        }
        if new != old && self.envs.contains_key(new) {
            anyhow::bail!("“{new}” already exists");
        }
        let Some(ix) = self.envs.get_index_of(old) else {
            anyhow::bail!("no environment “{old}”");
        };
        let vars = self.envs.shift_remove(old).unwrap_or_default();
        self.envs.shift_insert(ix, new.to_string(), vars);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) {
        self.envs.shift_remove(name);
    }

    /// `self` with `overrides` on top: environments are matched by name, and
    /// variables in `overrides` replace those of the same name.
    pub fn merged(&self, overrides: &Environments) -> Environments {
        let mut envs = self.envs.clone();
        for (name, vars) in &overrides.envs {
            envs.entry(name.clone())
                .or_default()
                .extend(vars.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        Environments { envs }
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

    #[test]
    fn edits_and_saves() {
        let dir = std::env::temp_dir().join(format!("reqo-test-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut envs = Environments::default();
        assert_eq!(envs.add("dev"), "dev");
        assert_eq!(envs.add("dev"), "dev-2");
        envs.set_variables(
            "dev",
            Variables::from_iter([("base".into(), "http://x".into())]),
        );
        assert!(envs.rename("dev-2", "dev").is_err());
        assert!(envs.rename("dev-2", " ").is_err());
        envs.rename("dev", "local").unwrap();
        // Renaming keeps the position.
        assert_eq!(envs.names().collect::<Vec<_>>(), ["local", "dev-2"]);
        envs.save(&dir).unwrap();

        let loaded = Environments::load(&dir).unwrap();
        assert_eq!(loaded.names().collect::<Vec<_>>(), ["local", "dev-2"]);
        assert_eq!(loaded.variables("local").unwrap()["base"], "http://x");
        let mut loaded = loaded;
        loaded.remove("local");
        assert_eq!(loaded.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn merged_overrides_by_name() {
        let base: Environments = Environments {
            envs: toml::from_str("[dev]\nbase = \"a\"\ntoken = \"t\"\n[prod]\nbase = \"p\"")
                .unwrap(),
        };
        let overrides = Environments {
            envs: toml::from_str("[dev]\nbase = \"b\"\n[local]\nbase = \"l\"").unwrap(),
        };
        let merged = base.merged(&overrides);
        assert_eq!(merged.names().collect::<Vec<_>>(), ["dev", "prod", "local"]);
        let dev = merged.variables("dev").unwrap();
        assert_eq!(dev["base"], "b");
        assert_eq!(dev["token"], "t");
    }
}
