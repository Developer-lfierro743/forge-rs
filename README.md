# Forge.rs 🔥

The Rust-native modloader ecosystem for Horizon — Minecraft: Java Edition rewritten in Rust.

## What is this?

Horizon already has `horizon-mixins` for injecting code and the launcher for building `.hnmod` files. What's missing is the **ecosystem layer**:

- **Dependency resolution** — "Mod A needs Mod B ^1.2"
- **Load ordering** — who wins when multiple mods hook the same function
- **Conflict detection** — catch broken modpacks before they crash
- **Modpack management** — `.fpack` bundles with manifests
- **Registry client** — publish and discover mods

That's what Forge.rs builds.

## Commands

```bash
# List all mods in a directory
forge-rs list --path ./mods

# Resolve dependency graph and print load order
forge-rs resolve --path ./mods

# Check for conflicts, missing deps, duplicate IDs
forge-rs check --path ./mods

# Scaffold a new mod project
forge-rs init my-mod --path ./my-mod
```

## Status

🚧 Early alpha. Manifest parsing and dependency resolution work. Build pipeline and registry coming next.

## Roadmap

- [x] `horizon-mixin.toml` manifest parsing
- [x] Recursive directory scanning
- [x] Dependency graph resolution with cycle detection
- [x] Load ordering by priority
- [x] Conflict detection (duplicates, missing deps)
- [ ] `forge-rs init` — scaffold a new `.hnmod` project
- [ ] `forge-rs build` — compile and package a mod
- [ ] `forge-rs pack` — create `.fpack` modpacks
- [ ] Registry client (`forge-rs publish`)
- [ ] Runtime mod management via Horizon launcher API

## License

MIT
