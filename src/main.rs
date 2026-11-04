use clap::{Parser, Subcommand};
use forge_rs::{
    build_hnmod, init_mod, pack_modpack, InstalledModsManager, ModpackManifest, ModRegistry,
    RegistryClient,
};
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "forge-rs")]
#[command(about = "Forge.rs — the Rust-native modloader for Horizon", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List installed/available mods
    List {
        /// Path to mods directory
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },
    /// Resolve dependency graph and load order
    Resolve {
        /// Path to mods directory
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },
    /// Check for conflicts between mods
    Check {
        /// Path to mods directory
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },
    /// Initialize a new Horizon native mod project
    Init {
        /// Mod ID (used in manifest and code)
        id: String,
        /// Output directory for the new mod project
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
        /// Overwrite existing directory
        #[arg(long)]
        force: bool,
    },
    /// Build a .hnmod from a mod project directory
    Build {
        /// Path to mod project directory (must contain horizon-mixin.toml)
        path: PathBuf,
        /// Output .hnmod file path
        #[arg(short, long, default_value = "mod.hnmod")]
        output: PathBuf,
        /// Overwrite existing output file
        #[arg(long)]
        force: bool,
        /// Horizon commit to pin (default: read from manifest)
        #[arg(long)]
        horizon_commit: Option<String>,
    },
    /// Create a .fpack modpack from a directory with modpack.toml
    Pack {
        /// Path to modpack directory (must contain modpack.toml)
        path: PathBuf,
        /// Output .fpack file path
        #[arg(short, long, default_value = "modpack.fpack")]
        output: PathBuf,
        /// Overwrite existing output file
        #[arg(long)]
        force: bool,
    },
    /// Validate a .fpack modpack
    CheckPack {
        /// Path to .fpack file
        path: PathBuf,
    },
    /// Manage installed mods
    Mods {
        #[command(subcommand)]
        command: ModsCommands,
    },
    /// Download and install a mod from the registry into the mods folder
    Registry {
        #[command(subcommand)]
        command: RegistryCommands,
    },
    /// Install a mod from the registry directly into the mods folder
    Install {
        /// Mod ID to install from registry
        mod_id: String,
        /// Mods directory
        #[arg(short, long, default_value = "mods")]
        path: PathBuf,
        /// Install as disabled
        #[arg(long)]
        disable: bool,
        /// Overwrite existing file
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum ModsCommands {
    /// List installed mods
    List {
        /// Path to mods directory
        #[arg(short, long, default_value = "mods")]
        path: PathBuf,
    },
    /// Install a .hnmod/.hmod file
    Install {
        /// Path to .hnmod/.hmod file
        source: PathBuf,
        /// Mods directory
        #[arg(short, long, default_value = "mods")]
        path: PathBuf,
        /// Install as disabled
        #[arg(long)]
        disable: bool,
    },
    /// Uninstall a mod
    Uninstall {
        /// Mod ID to uninstall
        mod_id: String,
        /// Mods directory
        #[arg(short, long, default_value = "mods")]
        path: PathBuf,
    },
    /// Enable a disabled mod
    Enable {
        /// Mod ID to enable
        mod_id: String,
        /// Mods directory
        #[arg(short, long, default_value = "mods")]
        path: PathBuf,
    },
    /// Disable an enabled mod
    Disable {
        /// Mod ID to disable
        mod_id: String,
        /// Mods directory
        #[arg(short, long, default_value = "mods")]
        path: PathBuf,
    },
}

#[derive(Subcommand)]
enum RegistryCommands {
    /// List all available mods from the Horizon registry
    List,
    /// Search for mods by name, ID, or description
    Search {
        /// Search query
        query: String,
    },
    /// Download and install a mod from the registry
    Install {
        /// Mod ID to install
        mod_id: String,
        /// Output directory (default: ./mods)
        #[arg(short, long, default_value = "mods")]
        output: PathBuf,
        /// Overwrite existing file
        #[arg(long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::List { path } => {
            let mut registry = ModRegistry::new();
            registry.load_dir(&path)?;
            let mods = registry.list();

            if mods.is_empty() {
                println!("No mods found in {}", path.display());
                return Ok(());
            }

            println!("{:<20} {:<10} {:<8} {}", "ID", "VERSION", "PRIORITY", "PATH");
            println!("{}", "-".repeat(60));
            for m in &mods {
                println!(
                    "{:<20} {:<10} {:<8} {}",
                    m.manifest.mod_.id,
                    m.manifest.mod_.version,
                    m.priority(),
                    m.path.display()
                );
            }
            println!("\nTotal: {} mods", mods.len());
        }

        Commands::Resolve { path } => {
            let mut registry = ModRegistry::new();
            registry.load_dir(&path)?;
            let resolved = registry.resolve()?;

            println!("Resolved load order (highest priority first):");
            println!("{}", "=".repeat(60));
            for (i, m) in resolved.iter().enumerate() {
                println!(
                    "{:2}. [{:>4}] {} v{}",
                    i + 1,
                    m.priority(),
                    m.manifest.mod_.id,
                    m.manifest.mod_.version
                );
                for dep in &m.manifest.dependencies {
                    println!("       └─ depends on: {} {:?}", dep.id, dep.version);
                }
            }
        }

        Commands::Check { path } => {
            let mut registry = ModRegistry::new();
            registry.load_dir(&path)?;

            println!("Checking for conflicts...");
            let mods: Vec<_> = registry.list().to_vec();

            // Check for duplicate IDs
            let mut seen = std::collections::HashMap::new();
            for m in &mods {
                seen.entry(&m.manifest.mod_.id)
                    .and_modify(|existing: &mut usize| *existing += 1)
                    .or_insert(1);
            }
            let duplicates: Vec<_> = seen.iter().filter(|(_, c)| **c > 1).collect();
            if !duplicates.is_empty() {
                println!("\n⚠ Duplicate mod IDs found:");
                for (id, count) in &duplicates {
                    println!("  {} ({} instances)", id, count);
                }
            } else {
                println!("✓ No duplicate mod IDs");
            }

            // Check dependency satisfaction
            let mut missing = Vec::new();
            for m in &mods {
                for dep in &m.manifest.dependencies {
                    if !registry.has_mod(&dep.id) {
                        missing.push((&m.manifest.mod_.id, &dep.id));
                    }
                }
            }

            if missing.is_empty() {
                println!("✓ All dependencies satisfied");
            } else {
                println!("\n⚠ Missing dependencies:");
                for (mod_id, dep_id) in &missing {
                    println!("  {} needs {}", mod_id, dep_id);
                }
            }

            if duplicates.is_empty() && missing.is_empty() {
                println!("\n✅ Mod pack looks good!");
            }
        }

        Commands::Init { id, path, force } => {
            let mod_dir = init_mod(&id, &path, force)?;
            println!("\n🎉 Mod project ready at: {}", mod_dir.display());
        }

        Commands::Build { path, output, force, horizon_commit } => {
            println!("Building .hnmod from {}...", path.display());
            let built = build_hnmod(&path, &output, force, horizon_commit.as_deref())?;
            println!("\n✅ Built: {}", built.display());
        }

        Commands::Pack { path, output, force } => {
            println!("Packing modpack from {}...", path.display());
            let pack_path = pack_modpack(&path, &output, force)?;
            println!("✅ Modpack ready: {}", pack_path.display());
        }

        Commands::CheckPack { path } => {
            println!("Checking modpack: {}", path.display());
            // Extract and validate
            let temp_dir = std::env::temp_dir().join("forge-rs-check");
            let _ = fs::remove_dir_all(&temp_dir);
            fs::create_dir_all(&temp_dir)?;

            let file = fs::File::open(&path)?;
            let mut zip = zip::ZipArchive::new(file)?;
            zip.extract(&temp_dir)?;

            let manifest = ModpackManifest::from_dir(&temp_dir)?;
            println!("  ID:          {}", manifest.id);
            println!("  Name:        {}", manifest.name);
            println!("  Version:     {}", manifest.version);
            if let Some(desc) = manifest.description {
                println!("  Description: {}", desc);
            }
            println!("  Mods:        {}", manifest.mods.len());
            for m in &manifest.mods {
                println!("    - {} v{} ({})", m.id, m.version, m.source);
            }

            // Check all mods are present
            for m in &manifest.mods {
                let mod_path = temp_dir.join(&m.path);
                if !mod_path.exists() {
                    println!("⚠ Missing mod file: {}", m.path);
                }
            }

            println!("\n✅ Modpack looks valid!");
        }

        Commands::Mods { command } => {
            let _ = std::env::current_dir()?; // mods_dir unused;
            match command {
                ModsCommands::List { path } => {
                    let mut manager = InstalledModsManager::new(path);
                    manager.load()?;
                    manager.list();
                }
                ModsCommands::Install { source, path, disable } => {
                    if !source.exists() {
                        anyhow::bail!("Source file not found: {}", source.display());
                    }
                    let mut manager = InstalledModsManager::new(path);
                    manager.load()?;
                    // Extract real mod ID from .hnmod manifest and rename to mod_id.hnmod
                    let mod_id = if let Ok(mut archive) = zip::ZipArchive::new(std::fs::File::open(&source).unwrap()) {
                        let mut found = None;
                        for i in 0..archive.len() {
                            if let Ok(mut entry) = archive.by_index(i) {
                                let name = entry.name();
                                if let Ok(name_str) = name {
                                    if name_str.ends_with("horizon-mixin.toml") {
                                        let mut buf = Vec::new();
                                        std::io::copy(&mut entry, &mut buf).unwrap();
                                        let toml_str = String::from_utf8_lossy(&buf);
                                        if let Some(line) = toml_str.lines().find(|l| l.trim().starts_with("id =")) {
                                            let id = line.split('=').nth(1).unwrap_or("").trim().trim_matches('"').to_lowercase();
                                            found = Some(id);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        found.unwrap_or_else(|| source.file_stem().unwrap_or_default().to_string_lossy().to_lowercase())
                    } else {
                        source.file_stem().unwrap_or_default().to_string_lossy().to_lowercase()
                    };
                    // Copy to a temp file with canonical name, then install that
                    let ext = source.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                    let canonical = source.with_file_name(format!("{}{}", mod_id, ext));
                    if !canonical.exists() {
                        fs::copy(&source, &canonical)?;
                    }
                    manager.install(&canonical, !disable)?;
                }
                ModsCommands::Uninstall { mod_id, path } => {
                    let mut manager = InstalledModsManager::new(path);
                    manager.uninstall(&mod_id)?;
                }
                ModsCommands::Enable { mod_id, path } => {
                    let mut manager = InstalledModsManager::new(path);
                    manager.enable(&mod_id)?;
                }
                ModsCommands::Disable { mod_id, path } => {
                    let mut manager = InstalledModsManager::new(path);
                    manager.disable(&mod_id)?;
                }
            }
        }

        Commands::Registry { command } => {
            let client = RegistryClient::horizon();
            match command {
                RegistryCommands::List => {
                    println!("Fetching Horizon mod registry...\n");
                    let mods = client.list_mods().await?;
                    if mods.is_empty() {
                        println!("No mods found in registry.");
                        return Ok(());
                    }
                    println!("{:<20} {:<15} {:<10} {}", "ID", "VERSION", "SIZE", "NAME");
                    println!("{}", "-".repeat(70));
                    for m in &mods {
                        let size_kb = m.file_size as f64 / 1024.0;
                        println!(
                            "{:<20} {:<15} {:<10.1}K {}",
                            m.id, m.version, size_kb, m.name
                        );
                    }
                    println!("\nTotal: {} mods", mods.len());
                }
                RegistryCommands::Search { query } => {
                    println!("Searching for: {}\n", query);
                    let results = client.search(&query).await?;
                    if results.is_empty() {
                        println!("No mods matched your search.");
                        return Ok(());
                    }
                    println!("{:<20} {:<15} {}", "ID", "VERSION", "NAME");
                    println!("{}", "-".repeat(60));
                    for m in &results {
                        println!("{:<20} {:<15} {}", m.id, m.version, m.name);
                    }
                    println!("\nFound: {} mods", results.len());
                }
                RegistryCommands::Install { mod_id, output, force } => {
                    println!("Looking up mod: {}\n", mod_id);
                    let all = client.list_mods().await?;
                    let mod_info = all.into_iter().find(|m| m.id == mod_id);
                    let Some(mod_info) = mod_info else {
                        anyhow::bail!("Mod '{}' not found in registry", mod_id);
                    };
                    println!("Found: {} v{}\n", mod_info.name, mod_info.version);
                    fs::create_dir_all(&output)?;
                    let dest = client.install_mod(&mod_info, &output, force).await?;
                    println!("\n✅ Downloaded to: {}", dest.display());
                }
            }
        }

        Commands::Install { mod_id, path, disable, force } => {
            println!("Installing '{}' from Horizon registry...\n", mod_id);
            let client = RegistryClient::horizon();

            // Lookup
            let all = client.list_mods().await?;
            let mod_info = all.into_iter().find(|m| m.id == mod_id);
            let Some(mod_info) = mod_info else {
                anyhow::bail!("Mod '{}' not found in registry", mod_id);
            };
            println!("Found: {} v{} ({} bytes)", mod_info.name, mod_info.version, mod_info.file_size);

            // Download to temp
            let temp_dir = std::env::temp_dir().join("forge-rs-install");
            fs::create_dir_all(&temp_dir)?;
            let downloaded = client.install_mod(&mod_info, &temp_dir, true).await?;

            // Rename to canonical {mod_id}.hnmod
            let ext = downloaded.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            let canonical = downloaded.with_file_name(format!("{}{}", mod_id, ext));
            if downloaded != canonical {
                let _ = fs::rename(&downloaded, &canonical);
            }

            // Install to mods folder via manager (canonical name + enable/disable)
            let mut manager = InstalledModsManager::new(path);
            manager.load()?;

            // Check if mod already exists (unless --force)
            let target_dir = if disable { manager.mods_dir.join("disabled") } else { manager.mods_dir.join("enabled") };
            let dest_path = target_dir.join(format!("{}{}", mod_id, ext));
            if dest_path.exists() && !force {
                anyhow::bail!(
                    "Mod '{}' already exists at {}. Use --force to overwrite.",
                    mod_id, dest_path.display()
                );
            }

            let dest = manager.install(&canonical, !disable)?;

            // Cleanup temp
            let _ = fs::remove_file(&canonical);

            println!("\n✅ Installed: {}", mod_info.id);
            if disable {
                println!("   Status: disabled");
            } else {
                println!("   Status: enabled");
            }
            println!("   Path:   {}", dest.display());
        }
    }

    Ok(())
}
