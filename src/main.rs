use clap::{Parser, Subcommand};
use forge_rs::ModRegistry;
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
    /// Initialize a new mod project
    Init {
        /// Mod ID
        id: String,
        /// Output directory
        #[arg(short, long, default_value = ".")]
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

        Commands::Init { id, path } => {
            println!("Initializing new mod project: {}", id);
            println!("Target directory: {}", path.display());
            println!("\nTODO: generate horizon-mixin.toml template and src/ structure");
        }
    }

    Ok(())
}
