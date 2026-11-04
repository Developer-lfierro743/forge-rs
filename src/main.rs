use clap::{Parser, Subcommand};
use forge_rs::{init_mod, pack_modpack, ModpackManifest, ModRegistry};
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
}

fn main() -> anyhow::Result<()> {
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
                println!("\n✓ Mod pack looks good!");
            }
        }

        Commands::Init { id, path, force } => {
            let mod_dir = init_mod(&id, &path, force)?;
            println!("\n🎉 Mod project ready at: {}", mod_dir.display());
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
    }

    Ok(())
}
