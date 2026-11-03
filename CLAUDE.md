# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview & Architecture

Dedupus is a high-performance duplicate file finder and disk cleaner inspired by [czkawka](https://github.com/qarmin/czkawka). It decouples the core file scanning/hashing engine from the user interfaces.

### Target Workspace Structure
- **`crates/dedupus_core/`**: Core scanning and detection engine in Rust.
  - Multi-stage pipeline: Directory walk ➔ Size-based grouping ➔ Prefix/Partial hash check (e.g. 4KB header) ➔ Full hash (BLAKE3 / XXH3).
  - Parallelism: `rayon` for concurrent I/O and hash crunching; `memmap2` for large file reads.
  - Media modules: Perceptual hashing (`image_hasher` / dHash / pHash) for similar images, acoustic fingerprinting for audio, frame sampling for video.
  - Scanning primitives: Cancellation tokens (`AtomicBool` / `tokio::sync::watch`), real-time progress callbacks and event streams.
- **`crates/dedupus_cli/`**: Standalone command-line interface powered by `clap`.
- **`src-tauri/`**: Tauri v2 desktop application backend (interfacing with `dedupus_core`).
- **`src/`** or **`frontend/`**: Modern Web-based UI built with Vite + React 19 + TypeScript + Tailwind CSS + Radix/shadcn.
  - Virtualization via `@tanstack/react-virtual` to seamlessly display lists with >100,000 items.

### Comparison & Modernization vs. Czkawka
- **GUI Framework**: Czkawka uses GTK4 / Slint; Dedupus adopts Tauri v2 + React 19 for seamless cross-platform desktop integration (native WebView2 on Windows) without bulky GTK dependencies.
- **Hashing**: Leverages BLAKE3 and XXH3-128 for maximum multi-core CPU throughput.
- **Safety**: Safe deletion defaults (system Trash/Recycle Bin via native OS APIs), hardlink deduplication, dry-run previews.

## Common Commands

### Rust / Core
```bash
# Check workspace
cargo check --workspace

# Build all packages
cargo build --workspace

# Build release
cargo build --release

# Run unit & integration tests
cargo test --workspace

# Run a single test
cargo test -p dedupus_core <test_name> -- --nocapture

# Clippy linter
cargo clippy --workspace --all-targets -- -D warnings

# Format check
cargo fmt --all -- --check
```

### CLI Commands
```bash
# Duplicate search
cargo run -p dedupus_cli -- dup <directory_path> [--algo blake3|xxh3_64|xxh3_128] [--min-size 1024]

# Similar image search (perceptual hashing)
cargo run -p dedupus_cli -- image <directory_path> [--distance 4] [--algo gradient|average]

# Similar audio / video search (acoustic envelope + keyframe dHash; video frames require ffmpeg on PATH)
cargo run -p dedupus_cli -- audio <directory_path> [--duration-diff 2.0]
cargo run -p dedupus_cli -- video <directory_path> [--duration-diff 2.0]

# Empty files & folders
cargo run -p dedupus_cli -- empty-files <directory_path>
cargo run -p dedupus_cli -- empty-folders <directory_path>

# Move files to system Recycle Bin / Trash
cargo run -p dedupus_cli -- trash <file_paths...>

# Cache management
cargo run -p dedupus_cli -- cache stats
cargo run -p dedupus_cli -- cache prune
cargo run -p dedupus_cli -- cache clear
```

### Frontend & Tauri
```bash
# Install frontend dependencies
pnpm install

# Run frontend in browser dev mode
pnpm dev

# Run Tauri desktop app in dev mode
pnpm tauri dev

# Typecheck and lint frontend
pnpm tauri:check || pnpm typecheck
pnpm lint

# Build production desktop bundle
pnpm tauri build
```
