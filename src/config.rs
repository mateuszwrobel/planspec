//! Project profile: how a spec's evidence resolves against the project.
//!
//! Shape (as TOML):
//!
//! ```toml
//! trunk_ref = "main"                 # git ref that commit cites must be an ancestor of
//! check_spec = "checks.toml"         # TOML file with `checks = ["name", ...]`
//! log_dir = "logs"                   # log cites resolve as files here
//! log_roots = ["logs", "workplans"]  # directories structured file cites may resolve under
//! sentinel_glob = "sentinel/{id}.done"  # sentinel cites resolve via this glob ({id} expands)
//! id_grammar = "^[a-z0-9][a-z0-9.-]*$" # regex every plan/task/tombstone id must match
//! ```

use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Deserialize;

use crate::model::SpecError;

/// Project profile file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Profile {
    #[serde(default = "default_trunk_ref")]
    pub trunk_ref: String,
    #[serde(default = "default_check_spec")]
    pub check_spec: PathBuf,
    #[serde(default = "default_log_dir")]
    pub log_dir: PathBuf,
    /// Directories structured file cites may resolve under, besides
    /// `log_dir` itself. A `file` cite outside every root is stale:
    /// the roots are the allowlist that keeps path cites inside the
    /// declared log-cite surface of the project. An empty list (the
    /// default, and every v0.1 profile) leaves file cites resolved by
    /// exact path alone — only the root-escape refusal applies.
    #[serde(default)]
    pub log_roots: Vec<PathBuf>,
    #[serde(default)]
    pub sentinel_glob: String,
    #[serde(default = "default_id_grammar")]
    pub id_grammar: String,
}

fn default_trunk_ref() -> String {
    "HEAD".to_string()
}

fn default_check_spec() -> PathBuf {
    PathBuf::from("checks.toml")
}

fn default_log_dir() -> PathBuf {
    PathBuf::from("logs")
}

fn default_id_grammar() -> String {
    r"^[A-Za-z0-9][A-Za-z0-9._-]*$".to_string()
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            trunk_ref: default_trunk_ref(),
            check_spec: default_check_spec(),
            log_dir: default_log_dir(),
            log_roots: Vec::new(),
            sentinel_glob: String::new(),
            id_grammar: default_id_grammar(),
        }
    }
}

impl Profile {
    /// Parse a project profile file. Problems are `Profile` errors
    /// (CLI exit 70).
    pub fn load(path: &Path) -> Result<Self, SpecError> {
        let text = std::fs::read_to_string(path).map_err(|e| SpecError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        toml::from_str(&text).map_err(|e| SpecError::Profile(e.to_string()))
    }

    /// Compile the id grammar. An invalid regex is a profile error
    /// (CLI exit 70).
    pub fn grammar(&self) -> Result<Regex, SpecError> {
        Regex::new(&self.id_grammar).map_err(|e| {
            SpecError::Profile(format!("id-grammar '{}': {e}", self.id_grammar))
        })
    }

    /// The check list named by `check_spec`, resolved against `root`.
    /// A missing or unreadable file is an error, so a check cite can
    /// never resolve silently.
    pub fn checks_at(&self, root: &Path) -> Result<Vec<String>, SpecError> {
        let path = root.join(&self.check_spec);
        let text =
            std::fs::read_to_string(&path).map_err(|e| SpecError::Io {
                path: path.display().to_string(),
                source: e,
            })?;
        let spec: CheckSpec =
            toml::from_str(&text).map_err(|e| SpecError::Profile(e.to_string()))?;
        Ok(spec.checks)
    }
}

/// The check-spec file referenced by a profile.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSpec {
    pub checks: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_profile() {
        let text = r#"
trunk-ref = "main"
check-spec = "c.toml"
log-dir = "worklog"
log-roots = ["worklog", "docs/log"]
sentinel-glob = "sentinel/{id}.done"
id-grammar = "^[a-z-]+$"
"#;
        let profile: Profile = toml::from_str(text).unwrap();
        assert_eq!(profile.trunk_ref, "main");
        assert_eq!(profile.check_spec, PathBuf::from("c.toml"));
        assert_eq!(profile.log_dir, PathBuf::from("worklog"));
        assert_eq!(
            profile.log_roots,
            [PathBuf::from("worklog"), PathBuf::from("docs/log")]
        );
        assert_eq!(profile.sentinel_glob, "sentinel/{id}.done");
        assert_eq!(profile.id_grammar, "^[a-z-]+$");
    }

    #[test]
    fn fields_default() {
        let profile: Profile = toml::from_str("").unwrap();
        let default = Profile::default();
        assert_eq!(profile.trunk_ref, default.trunk_ref);
        assert_eq!(profile.check_spec, default.check_spec);
        assert_eq!(profile.log_dir, default.log_dir);
        assert!(profile.log_roots.is_empty());
        assert_eq!(profile.sentinel_glob, "");
        assert_eq!(profile.id_grammar, default.id_grammar);
    }

    #[test]
    fn unknown_profile_field_is_invalid() {
        assert!(toml::from_str::<Profile>("bogus = 1").is_err());
    }

    #[test]
    fn check_spec_file_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checks.toml");
        std::fs::write(&path, "checks = [\"clippy\", \"nextest\"]\n").unwrap();
        let profile = Profile {
            check_spec: path,
            ..Profile::default()
        };
        assert_eq!(
            profile.checks_at(Path::new(".")).unwrap(),
            ["clippy".to_string(), "nextest".to_string()]
        );
    }

    #[test]
    fn missing_check_spec_is_error() {
        let profile = Profile::default();
        assert!(profile.checks_at(Path::new("/nonexistent-dir-planspec")).is_err());
    }
}
