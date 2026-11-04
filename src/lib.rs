use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
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
    #[serde(default)]
    pub modules: Vec<ModuleEntry>,
    #[serde(default)]
    pub injections: Vec<InjectionEntry>,
    #[serde(default)]
    pub post_build: Vec<PostBuildEntry>,
    #[serde(default)]
    pub crates: Vec<CrateEntry>,
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
}

#[derive(Debug, Deserialize, Clone)]
pub struct Dependency {
    pub id: String,
    pub version: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModuleEntry {
    pub id: String,
    pub file: String,
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub visibility: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Binding {
    pub name: String,
    #[serde(rename = "ty")]
    pub ty: String,
    pub value: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct InjectionEntry {
    pub id: String,
    #[serde(rename = "kind")]
    pub kind: String,
    pub target: Option<String>,
    pub hook: Option<String>,
    #[serde(default)]
    pub hook_version: Option<u32>,
    pub helper: String,
    pub bindings: Vec<Binding>,
    #[serde(default)]
    pub cancellable: Option<bool>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PostBuildEntry {
    pub crate_: String,
    pub bin: String,
    pub args: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CrateEntry {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub dependent: Option<String>,
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

pub fn init_mod(id: &str, path: &Path, force: bool) -> Result<PathBuf> {
    let sanitized = sanitize_id(id);
    let mod_dir = path.join(&sanitized);

    if mod_dir.exists() && !force {
        anyhow::bail!("Directory '{}' already exists. Use --force to overwrite.", mod_dir.display());
    }

    fs::create_dir_all(&mod_dir)?;
    let mixins_dir = mod_dir.join("mixins").join(&sanitized);
    fs::create_dir_all(&mixins_dir)?;

    write_file(&mod_dir, "horizon-mixin.toml", &manifest_template(&sanitized))?;
    write_file(&mod_dir, "README.md", &readme_template(&sanitized))?;
    write_file(&mod_dir, ".gitignore", &gitignore_template())?;

    write_file(&mixins_dir, "runtime.rs", &runtime_template(&sanitized))?;
    write_file(&mixins_dir, "tick.rs", &tick_template(&sanitized))?;
    write_file(&mixins_dir, "frame.rs", &frame_template(&sanitized))?;
    write_file(&mixins_dir, "hud.rs", &hud_template(&sanitized))?;
    write_file(&mixins_dir, "list.rs", &list_template(&sanitized))?;
    write_file(&mixins_dir, "command.rs", &command_template(&sanitized))?;
    write_file(&mixins_dir, "settings.rs", &settings_template(&sanitized))?;

    println!("✓ Created mod project: {}", mod_dir.display());
    println!("  {}", mod_dir.join("horizon-mixin.toml").display());
    println!("\nNext steps:");
    println!("  1. Edit mixins/{}/runtime.rs — your mod's state", sanitized);
    println!("  2. Edit mixins/{}/tick.rs — per-tick logic", sanitized);
    println!("  3. Edit mixins/{}/hud.rs — HUD drawing", sanitized);
    println!("  4. Build with: horizon-launcher native-build {} --trust-native-code", mod_dir.display());
    println!("\nHook points used:");
    println!("  native_tick  native_frame  native_hud  native_mods");
    println!("  native_command  native_settings");

    Ok(mod_dir)
}

fn write_file(base: &Path, rel: &str, content: &str) -> Result<()> {
    let path = base.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(())
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c.to_ascii_lowercase() } else { '_' })
        .collect::<String>()
        .trim_start_matches('_')
        .to_string()
}

fn manifest_template(id: &str) -> String {
    format!(
r#"[mod]
id = "{id}"
version = "0.1.0"
priority = 100

[compatibility]
horizon_revision = "latest"

[[module]]
id = "runtime"
file = "crates/horizon-client/src/game.rs"
name = "{id}"
source = "runtime.rs"

# Uncomment to hook game functions directly (no hook point needed):
# [[injection]]
# id = "before-break"
# kind = "HEAD"
# target = "horizon_client::game::Game::break_block"
# helper = "break.rs"
# bindings = [{{ name = "pos", ty = "[i32;3]", value = "pos" }}]

# ─── Hook points ───────────────────────────────────────────
# See docs at playhorizon.dev/docs/mod-code

# Per-tick logic (20 times/sec while in a world)
[[injection]]
id = "tick"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game::native_tick"
helper = "tick.rs"
bindings = [
  {{ name = "game", ty = "&mut crate::game::Game", value = "&mut *self" }},
]

# Per-frame logic (every rendered frame)
[[injection]]
id = "frame"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game::native_frame"
helper = "frame.rs"
bindings = [
  {{ name = "game", ty = "&crate::game::Game", value = "&*self" }},
  {{ name = "elapsed", ty = "std::time::Duration", value = "elapsed" }},
]

# HUD drawing
[[injection]]
id = "hud"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game::native_hud"
helper = "hud.rs"
bindings = [
  {{ name = "game", ty = "&mut crate::game::Game", value = "&mut *self" }},
  {{ name = "batch", ty = "&mut HudBatch", value = "&mut *batch" }},
]

# Options > Mods list entry
[[injection]]
id = "list"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game::native_mods"
helper = "list.rs"
bindings = [
  {{ name = "mods", ty = "&mut Vec<crate::game::native_hooks::NativeMod>", value = "&mut *mods" }},
]

# Chat command (without the slash)
[[injection]]
id = "command"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game::native_command"
helper = "command.rs"
bindings = [
  {{ name = "game", ty = "&mut crate::game::Game", value = "&mut *self" }},
  {{ name = "command", ty = "&str", value = "command" }},
  {{ name = "ran", ty = "&mut bool", value = "&mut *ran" }},
]

# Settings page buttons
[[injection]]
id = "settings"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game::native_settings"
helper = "settings.rs"
bindings = [
  {{ name = "game", ty = "&mut crate::game::Game", value = "&mut *self" }},
  {{ name = "mod_id", ty = "&str", value = "mod_id" }},
  {{ name = "buttons", ty = "&mut Vec<String>", value = "&mut *buttons" }},
]

# Settings button click
[[injection]]
id = "settings_click"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game::native_settings_click"
helper = "settings.rs"
bindings = [
  {{ name = "game", ty = "&mut crate::game::Game", value = "&mut *self" }},
  {{ name = "mod_id", ty = "&str", value = "mod_id" }},
  {{ name = "button", ty = "usize", value = "button" }},
]
"#,
        id = id
    )
}

fn runtime_template(id: &str) -> String {
    format!(
r#"// {id} — mod state lives here.
//
// Each hook helper calls into this module. Keep mutable state in a
// thread_local! or in the module itself; the window's game runs on one
// thread, so the same instance is visible everywhere.

use std::cell::Cell;

thread_local! {{
    /// Ticks since the world was opened. Reset on world close if you need.
    pub static TICKS: Cell<u64> = Cell::new(0);
}}

/// Called once when the mod is loaded. Return an error to disable the mod.
pub fn init() -> anyhow::Result<()> {{
    Ok(())
}}

/// Reset per-world state.
pub fn on_world_open() {{
    TICKS.with(|t| t.set(0));
}}
"#,
        id = id
    )
}

fn tick_template(id: &str) -> String {
    format!(
r#"// Per-tick hook — runs 20 times/sec while in a world.
//
// Bindings: game: &mut Game<'_>

use crate::game::{{runtime, native_hooks}};

pub fn tick(game: &mut crate::game::Game<'_>) {{
    if !native_hooks::native_on("{id}") {{
        return; // mod turned off in Options > Mods
    }}

    // Count ticks
    runtime::TICKS.with(|t| {{
        t.set(t.get() + 1);
    }});

    // Your tick logic here:
    // - update state
    // - check timers
    // - write to the game world
}}
"#,
        id = id
    )
}

fn frame_template(id: &str) -> String {
    format!(
r#"// Per-frame hook — runs every rendered frame.
//
// Bindings: game: &Game<'_>, elapsed: Duration

use crate::game::runtime;

pub fn frame(_game: &crate::game::Game<'_>, _elapsed: std::time::Duration) {{
    if !crate::game::native_hooks::native_on("{id}") {{
        return;
    }}

    // Your frame logic here:
    // - animate HUD elements
    // - smooth camera effects
    // - update overlays
}}
"#,
        id = id
    )
}

fn hud_template(id: &str) -> String {
    format!(
r#"// HUD drawing hook — draw on screen here.
//
// Bindings:
//   game:  &mut Game<'_>
//   batch: &mut HudBatch

use crate::game::{{native_hooks, runtime}};
use crate::game::native_hooks::{{native_corner_shown, native_text_width, native_fill, native_text}};

pub fn hud(game: &mut crate::game::Game<'_>, batch: &mut crate::game::hud::HudBatch) {{
    if !native_hooks::native_on("{id}") {{
        return;
    }}

    // Example: draw a small box in the top-left corner
    let x = 4.0;
    let y = 4.0;
    let w = 120.0;
    let h = 24.0;

    // Background (argb: 0x78 = ~47% alpha)
    native_fill(batch, [x, y, w, h], 0x78000000);

    // Text — use your mod's actual message here
    let text = format!("{{id}} v0.1.0 — ticks: {{}}", runtime::TICKS.with(|t| t.get()));
    native_text(batch, &text, [x + 4.0, y + 4.0], 0xFFFFFFFF, true);
}}
"#,
        id = id
    )
}

fn list_template(id: &str) -> String {
    format!(
r#"// Options > Mods list entry — registers your mod in the mods menu.
//
// Bindings: mods: &mut Vec<NativeMod>

use crate::game::native_hooks::NativeMod;

pub fn list(mods: &mut Vec<NativeMod>) {{
    // Only add your row once; this hook runs for every mod, so check the id.
    if mods.iter().any(|m| m.id == "{id}") {{
        return;
    }}

    mods.push(NativeMod {{
        id: "{id}".into(),
        title: "{id} — Forge.rs mod".into(),
        description: "A starter Horizon native mod".into(),
        keys: vec![],
    }});
}}
"#,
        id = id
    )
}

fn command_template(id: &str) -> String {
    format!(
r#"// Chat command hook — respond to /<command> in chat.
//
// Bindings:
//   command: &str   (the message without the leading slash)
//   ran:     &mut bool  (set to true if you handled it)

pub fn command(_game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {{
    if !crate::game::native_hooks::native_on("{id}") {{
        return;
    }}

    let args: Vec<&str> = command.split_whitespace().collect();

    match args.get(0).copied() {{
        Some("{id}") => {{
            // /{id} — print a hello
            crate::game::native_hooks::native_chat(
                &format!("👋 Hello from {id} v0.1.0!"),
            );
            *ran = true;
        }}
        Some("{id}reload") => {{
            // /{id}reload — reload config if you have one
            crate::game::native_hooks::native_chat("{id} reloaded!");
            *ran = true;
        }}
        _ => {{}}
    }}
}}
"#,
        id = id
    )
}

fn settings_template(id: &str) -> String {
    format!(
r#"// Settings page — add buttons to Options > Mods > <your mod> > Settings.
//
// Bindings:
//   mod_id:   &str
//   buttons:  &mut Vec<String>

pub fn settings(_game: &mut crate::game::Game<'_>, mod_id: &str, buttons: &mut Vec<String>) {{
    if mod_id != "{id}" {{
        return;
    }}

    // Each string becomes a button. Clicking it calls settings_click with
    // the button's index.
    buttons.push("Reset counters".into());
    buttons.push("Toggle debug".into());
}}

// Button click handler.
pub fn settings_click(
    _game: &mut crate::game::Game<'_>,
    mod_id: &str,
    button: usize,
) {{
    if mod_id != "{id}" {{
        return;
    }}

    match button {{
        0 => {{
            // Reset counters button
            crate::game::runtime::TICKS.with(|t| t.set(0));
            crate::game::native_hooks::native_chat("{id}: counters reset");
        }}
        1 => {{
            // Toggle debug button
            crate::game::native_hooks::native_chat("{id}: debug toggled");
        }}
        _ => {{}}
    }}
}}
"#,
        id = id
    )
}

fn readme_template(id: &str) -> String {
    format!(
r#"# {id}

A Forge.rs mod for Horizon — Minecraft: Java Edition 26.3 rewritten in Rust.

## Building

```bash
# From your Horizon install, build the game with this mod:
horizon-launcher native-build . --trust-native-code
```

Or install it from the Horizon launcher's Mods page once published.

## What it does

- Registers itself in Options > Mods
- Draws a HUD element in the top-left corner
- Counts ticks and frames
- Responds to `/{id}` and `/{id}reload` in chat
- Has a settings page with two buttons

## Hook points used

| Hook | Purpose |
|------|---------|
| `native_tick` | Tick counter |
| `native_frame` | Frame timing |
| `native_hud` | Draw the HUD box |
| `native_mods` | Register in mods list |
| `native_command` | Chat commands |
| `native_settings` | Settings buttons |
| `native_settings_click` | Settings clicks |

## License

MIT
"#,
        id = id
    )
}

fn gitignore_template() -> String {
    r#"# Horizon native mod build artifacts
target/
horizon-mods/
*.hnmod
*.hmod
*.fpack

# Rust
Cargo.lock
**/*.rs.bk

# OS
.DS_Store
Thumbs.db
"#
    .to_string()
}

// ─── Modpack (.fpack) builder ───────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModpackManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub game: Option<String>,
    pub mods: Vec<ModpackModEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModpackModEntry {
    pub id: String,
    pub version: String,
    pub source: String,
    pub path: String,
}

impl ModpackManifest {
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let manifest_path = dir.join("modpack.toml");
        if !manifest_path.exists() {
            anyhow::bail!("No modpack.toml found in {}", dir.display());
        }
        let content = fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?;
        let manifest: ModpackManifest = toml::from_str(&content)
            .with_context(|| format!("parsing {}", manifest_path.display()))?;
        Ok(manifest)
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        let toml_str = toml::to_string(self)
            .context("serializing modpack manifest")?;
        fs::write(path, toml_str)?;
        Ok(())
    }
}

pub fn pack_modpack(modpack_dir: &Path, output: &Path, force: bool) -> Result<PathBuf> {
    if output.exists() && !force {
        anyhow::bail!("Output file '{}' already exists. Use --force to overwrite.", output.display());
    }

    let manifest = ModpackManifest::from_dir(modpack_dir)?;
    let mut entries: Vec<(PathBuf, String)> = Vec::new();

    // Add manifest
    entries.push((modpack_dir.join("modpack.toml"), "modpack.toml".to_string()));

    // Add each mod file/dir referenced in manifest
    for entry in &manifest.mods {
        let source_path = modpack_dir.join(&entry.source);
        if !source_path.exists() {
            anyhow::bail!("Mod source not found: {}", source_path.display());
        }
        if source_path.is_dir() {
            for file in walk_dir(&source_path)? {
                let rel = file.strip_prefix(&source_path).unwrap_or(&file);
                entries.push((file.clone(), format!("mods/{}/{}", entry.id, rel.display())));
            }
        } else {
            entries.push((source_path, format!("mods/{}", entry.source)));
        }
    }

    // Create zip
    let file = fs::File::create(output)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::<()>::default();

    for (src, dest) in &entries {
        let content = fs::read(src)?;
        zip.start_file(dest, options)?;
        zip.write_all(&content)?;
    }

    zip.finish()?;

    println!("✓ Packed {} mods into {}", manifest.mods.len(), output.display());
    println!("  {} ({} bytes)", manifest.id, fs::metadata(output)?.len());
    Ok(output.to_path_buf())
}

fn walk_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk_dir(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}

// ─── GitHub Registry Client ─────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub name: String,
    pub prerelease: bool,
    pub assets: Vec<GitHubAsset>,
    pub body: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GitHubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RegistryModInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub download_url: String,
    pub file_size: u64,
    pub prerelease: bool,
    pub published_at: Option<String>,
}

pub struct RegistryClient {
    client: reqwest::Client,
    owner: String,
    repo: String,
}

impl RegistryClient {
    pub fn new(owner: &str, repo: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
    }

    pub fn horizon() -> Self {
        Self::new("horizon-rust", "horizon-mods")
    }

    pub async fn list_mods(&self) -> Result<Vec<RegistryModInfo>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/releases",
            self.owner, self.repo
        );
        let releases: Vec<GitHubRelease> = self
            .client
            .get(&url)
            .header("User-Agent", "forge-rs")
            .send()
            .await?
            .json()
            .await?;

        let mut mods = Vec::new();
        for release in releases {
            for asset in release.assets {
                if asset.name.ends_with(".hnmod") || asset.name.ends_with(".hmod") {
                    let base = asset.name
                        .trim_end_matches(".hnmod")
                        .trim_end_matches(".hmod")
                        .to_string();
                    // Strip version suffix: "mod_menu-0.3.0" -> "mod_menu"
                    let mod_id = if let Some((name, _ver)) = base.rsplit_once('-') {
                        if _ver.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                            name.to_string()
                        } else {
                            base
                        }
                    } else {
                        base
                    };
                    mods.push(RegistryModInfo {
                        id: mod_id,
                        name: release.name.clone(),
                        version: release.tag_name.clone(),
                        description: release.body.clone(),
                        download_url: asset.browser_download_url.clone(),
                        file_size: asset.size,
                        prerelease: release.prerelease,
                        published_at: release.published_at.clone(),
                    });
                }
            }
        }

        Ok(mods)
    }

    pub async fn search(&self, query: &str) -> Result<Vec<RegistryModInfo>> {
        let all = self.list_mods().await?;
        let q = query.to_lowercase();
        Ok(all
            .into_iter()
            .filter(|m| {
                m.id.contains(&q)
                    || m.name.to_lowercase().contains(&q)
                    || m.description
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(&q))
                        .unwrap_or(false)
            })
            .collect())
    }

    pub async fn install_mod(
        &self,
        mod_info: &RegistryModInfo,
        dest_dir: &Path,
        force: bool,
    ) -> Result<PathBuf> {
        let filename = mod_info.download_url.rsplit('/').next().unwrap_or("mod.hnmod");
        let dest = dest_dir.join(filename);

        if dest.exists() && !force {
            anyhow::bail!(
                "Mod file '{}' already exists. Use --force to overwrite.",
                dest.display()
            );
        }

        println!("Downloading {} v{}...", mod_info.id, mod_info.version);
        let bytes = self
            .client
            .get(&mod_info.download_url)
            .header("User-Agent", "forge-rs")
            .send()
            .await?
            .bytes()
            .await?;

        fs::create_dir_all(dest_dir)?;
        fs::write(&dest, &bytes)?;

        println!(
            "✓ Downloaded {} ({} bytes)",
            dest.display(),
            bytes.len()
        );
        Ok(dest)
    }
}

// ─── .hnmod Builder ─────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HorizonNativeJson {
    pub id: String,
    pub version: String,
    pub game: String,
    pub horizon: String,
    pub mixins: String,
    pub schema: u32,
}

pub fn build_hnmod(
    mod_dir: &Path,
    output: &Path,
    force: bool,
    horizon_commit: Option<&str>,
) -> Result<PathBuf> {
    if output.exists() && !force {
        anyhow::bail!("Output '{}' already exists. Use --force to overwrite.", output.display());
    }

    // Load manifest
    let manifest_path = mod_dir.join("horizon-mixin.toml");
    if !manifest_path.exists() {
        anyhow::bail!("No horizon-mixin.toml found in {}", mod_dir.display());
    }
    let manifest = ModManifest::from_file(&manifest_path)?;

    let mod_id = &manifest.mod_.id;
    let version = &manifest.mod_.version;
    let horizon_rev = manifest
        .compatibility
        .as_ref()
        .and_then(|c| c.horizon_revision.clone())
        .or_else(|| horizon_commit.map(|s| s.to_string()))
        .unwrap_or_else(|| "latest".to_string());

    // Create zip
    let file = fs::File::create(output)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::<()>::default();

    // 1. horizon-native.json
    let native_json = HorizonNativeJson {
        id: mod_id.clone(),
        version: version.clone(),
        game: "26.3".to_string(),
        horizon: horizon_rev.clone(),
        mixins: "latest".to_string(),
        schema: 1,
    };
    let json_bytes = serde_json::to_string_pretty(&native_json)?;
    zip.start_file("horizon-native.json", options)?;
    zip.write_all(json_bytes.as_bytes())?;

    // 2. Copy mixins/<mod_id>/* from mod_dir
    let mixins_src = mod_dir.join("mixins").join(mod_id);
    if !mixins_src.exists() {
        anyhow::bail!("Mixins directory not found: {}", mixins_src.display());
    }

    let mut added_toml = false;
    for entry in walk_dir(&mixins_src)? {
        let rel = entry.strip_prefix(&mixins_src).unwrap_or(&entry);
        let dest = format!("mixins/{}/{}", mod_id, rel.display());
        let content = fs::read(&entry)?;
        zip.start_file(dest, options)?;
        zip.write_all(&content)?;
        if rel.to_string_lossy() == "horizon-mixin.toml" {
            added_toml = true;
        }
    }

    // 3. Include horizon-mixin.toml if not already in mixins dir
    if !added_toml {
        let toml_bytes = fs::read(&manifest_path)?;
        zip.start_file(format!("mixins/{}/horizon-mixin.toml", mod_id), options)?;
        zip.write_all(&toml_bytes)?;
    }

    zip.finish()?;

    let size = fs::metadata(output)?.len();
    println!("✓ Built {} v{}", mod_id, version);
    println!("  {} ({} bytes)", output.display(), size);
    println!("  horizon: {}", horizon_rev);
    println!("  game: 26.3");
    println!("  schema: 1");

    Ok(output.to_path_buf())
}

// ─── Installed Mods Manager ─────────────────────────────────

#[derive(Debug, Clone)]
pub struct InstalledModsManager {
    pub mods_dir: PathBuf,
    pub enabled: HashMap<String, PathBuf>,
    pub disabled: HashMap<String, PathBuf>,
}

impl InstalledModsManager {
    pub fn new(mods_dir: PathBuf) -> Self {
        Self {
            mods_dir,
            enabled: HashMap::new(),
            disabled: HashMap::new(),
        }
    }

    pub fn load(&mut self) -> Result<()> {
        self.enabled.clear();
        self.disabled.clear();

        let enabled_dir = self.mods_dir.join("enabled");
        let disabled_dir = self.mods_dir.join("disabled");

        if enabled_dir.exists() {
            for entry in fs::read_dir(enabled_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().map(|e| e == "hnmod" || e == "hmod").unwrap_or(false) {
                    if let Some(stem) = path.file_stem() {
                        let id = stem.to_string_lossy().to_lowercase();
                        self.enabled.insert(id, path);
                    }
                }
            }
        }

        if disabled_dir.exists() {
            for entry in fs::read_dir(disabled_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().map(|e| e == "hnmod" || e == "hmod").unwrap_or(false) {
                    if let Some(stem) = path.file_stem() {
                        let id = stem.to_string_lossy().to_lowercase();
                        self.disabled.insert(id, path);
                    }
                }
            }
        }

        Ok(())
    }

    pub fn install(&mut self, source: &Path, enable: bool) -> Result<PathBuf> {
        let dest_dir = if enable {
            self.mods_dir.join("enabled")
        } else {
            self.mods_dir.join("disabled")
        };

        fs::create_dir_all(&dest_dir)?;

        let filename = source.file_name().unwrap_or_default();
        let dest = dest_dir.join(filename);

        if dest.exists() {
            fs::remove_file(&dest)?;
        }

        fs::copy(source, &dest)?;
        let _ = self.load()?;

        println!("✓ Installed to {}", dest.display());
        Ok(dest)
    }

    pub fn uninstall(&mut self, mod_id: &str) -> Result<()> {
        let _ = self.load();

        if let Some(path) = self.enabled.remove(mod_id) {
            fs::remove_file(&path)?;
            println!("✓ Uninstalled enabled mod: {}", mod_id);
            return Ok(());
        }

        if let Some(path) = self.disabled.remove(mod_id) {
            fs::remove_file(&path)?;
            println!("✓ Uninstalled disabled mod: {}", mod_id);
            return Ok(());
        }

        anyhow::bail!("Mod '{}' not found", mod_id);
    }

    pub fn enable(&mut self, mod_id: &str) -> Result<()> {
        let _ = self.load();

        if let Some(path) = self.disabled.remove(mod_id) {
            let dest = self.mods_dir.join("enabled").join(
                path.file_name().unwrap_or_default()
            );
            fs::create_dir_all(self.mods_dir.join("enabled"))?;
            fs::rename(&path, &dest)?;
            let _ = self.load();
            println!("✓ Enabled mod: {}", mod_id);
            Ok(())
        } else if self.enabled.contains_key(mod_id) {
            println!("Mod '{}' is already enabled", mod_id);
            Ok(())
        } else {
            anyhow::bail!("Mod '{}' not found", mod_id);
        }
    }

    pub fn disable(&mut self, mod_id: &str) -> Result<()> {
        let _ = self.load();

        if let Some(path) = self.enabled.remove(mod_id) {
            let dest = self.mods_dir.join("disabled").join(
                path.file_name().unwrap_or_default()
            );
            fs::create_dir_all(self.mods_dir.join("disabled"))?;
            fs::rename(&path, &dest)?;
            let _ = self.load();
            println!("✓ Disabled mod: {}", mod_id);
            Ok(())
        } else if self.disabled.contains_key(mod_id) {
            println!("Mod '{}' is already disabled", mod_id);
            Ok(())
        } else {
            anyhow::bail!("Mod '{}' not found", mod_id);
        }
    }

    pub fn list(&self) {
        println!("Mods directory: {}\n", self.mods_dir.display());

        if !self.enabled.is_empty() {
            println!("Enabled mods ({}):", self.enabled.len());
            for (id, path) in &self.enabled {
                let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                println!("  ✓ {:<25} ({} bytes)", id, size);
            }
            println!();
        }

        if !self.disabled.is_empty() {
            println!("Disabled mods ({}):", self.disabled.len());
            for (id, path) in &self.disabled {
                let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                println!("  ✗ {:<25} ({} bytes)", id, size);
            }
            println!();
        }

        let total = self.enabled.len() + self.disabled.len();
        if total == 0 {
            println!("No mods installed.");
        }
    }
}
