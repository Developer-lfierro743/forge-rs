use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ForgeError {
    #[error("manifest parse error: {0}")]
    ManifestParse(#[from] toml::de::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("missing field: {field} in {file}")]
    MissingField { field: &'static str, file: String },
    #[error("dependency not found: {0} required by {1}")]
    MissingDependency(String, String),
    #[error("circular dependency: {0} <-> {1}")]
    CircularDependency(String, String),
    #[error("version conflict: {0} (needs {1}, found {2})")]
    VersionConflict(String, String, String),
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModManifest {
    #[serde(rename = "mod")]
    pub mod_: ModInfo,
    pub compatibility: Option<Compatibility>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModInfo {
    pub id: String,
    pub version: String,
    pub priority: Option<u32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Compatibility {
    pub horizon_revision: Option<String>,
    pub horizon_mixins: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Dependency {
    pub id: String,
    pub version: Option<String>,
}

impl ModManifest {
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut manifest: ModManifest = toml::from_str(&content)
            .with_context(|| format!("parsing {}", path.display()))?;
        manifest.normalize();
        Ok(manifest)
    }

    /// Normalize mod id (strip namespace, lowercase)
    fn normalize(&mut self) {
        self.mod_.id = self.mod_.id.to_lowercase();
        for dep in &mut self.dependencies {
            dep.id = dep.id.to_lowercase();
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedMod {
    pub manifest: ModManifest,
    pub path: PathBuf,
    pub enabled: bool,
}

impl LoadedMod {
    pub fn id(&self) -> &str {
        &self.manifest.mod_.id
    }

    pub fn priority(&self) -> u32 {
        self.manifest.mod_.priority.unwrap_or(100)
    }
}

pub struct ModRegistry {
    mods: HashMap<String, LoadedMod>,
}

impl ModRegistry {
    pub fn new() -> Self {
        Self {
            mods: HashMap::new(),
        }
    }

    pub fn load_dir(&mut self, dir: &Path) -> Result<()> {
        self.load_dir_recursive(dir)?;
        Ok(())
    }

    fn load_dir_recursive(&mut self, dir: &Path) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                self.load_dir_recursive(&path)?;
            } else if path.extension().map(|e| e == "toml").unwrap_or(false) {
                let manifest = ModManifest::from_file(&path)?;
                let id = manifest.mod_.id.clone();
                let loaded = LoadedMod {
                    manifest,
                    path: path.parent().unwrap_or(dir).to_path_buf(),
                    enabled: true,
                };
                self.mods.insert(id, loaded);
            }
        }
        Ok(())
    }

    pub fn resolve(&self) -> Result<Vec<&LoadedMod>> {
        let mut resolved = Vec::new();
        let mut visiting = std::collections::HashSet::new();

        for mod_ in self.mods.values() {
            if mod_.enabled {
                self.resolve_mod(mod_.id(), &mut resolved, &mut visiting)?;
            }
        }

        // Sort by priority descending (higher priority = loaded first)
        resolved.sort_by(|a, b| b.priority().cmp(&a.priority()));
        Ok(resolved)
    }

    fn resolve_mod<'a>(
        &'a self,
        id: &str,
        resolved: &mut Vec<&'a LoadedMod>,
        visiting: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        if resolved.iter().any(|m| m.id() == id) {
            return Ok(());
        }

        if !visiting.insert(id.to_string()) {
            return Err(ForgeError::CircularDependency(
                id.to_string(),
                visiting.iter().next().unwrap().clone(),
            )
            .into());
        }

        if let Some(mod_) = self.mods.get(id) {
            if !mod_.enabled {
                return Err(ForgeError::MissingDependency(
                    id.to_string(),
                    "disabled mod".to_string(),
                )
                .into());
            }

            for dep in &mod_.manifest.dependencies {
                self.resolve_mod(&dep.id, resolved, visiting)?;
            }

            resolved.push(mod_);
        }

        visiting.remove(id);
        Ok(())
    }

    pub fn list(&self) -> Vec<&LoadedMod> {
        self.mods.values().collect()
    }

    pub fn has_mod(&self, id: &str) -> bool {
        self.mods.contains_key(id)
    }
}
