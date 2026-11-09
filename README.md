# Kiln

A desktop app for saving, parameterizing, running, and syncing reusable shell
scripts. Scripts stay in a normal git repository; runs, settings, and history
stay local to the machine.

`SPEC.md` is the product source of truth.

## Requirements

- [Bun](https://bun.sh/) for the frontend toolchain.
- Rust and Cargo for the Tauri backend.
- The platform dependencies listed in the
  [Tauri prerequisites](https://tauri.app/start/prerequisites/).

## Development

```bash
bun install
bun run tauri dev
```

Build a production bundle:

```bash
bun run tauri build
```

Frontend-only checks:

```bash
bun run build
```

Rust tests and formatting:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
```

## Layout

- `src/` — React + TypeScript UI. The shell, typed IPC wrappers, and settings
  surface live here. It contains no run logic.
- `src-tauri/src/` — Rust backend. `db.rs` owns the SQLite file and migrations,
  `settings.rs` owns typed app settings, `commands.rs` is the Tauri IPC layer,
  and the remaining modules are stubs for the features described in `SPEC.md`.
- `src-tauri/` — Tauri configuration, capabilities, and the Cargo project.

## Local data

The SQLite database is created in the OS app data directory (never in the
script repository) and uses WAL mode. Migrations apply on startup and are
versioned.
