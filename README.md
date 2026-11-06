# Dedupus 🔍

> A blazing fast, memory-efficient, modern duplicate file finder and disk cleaner.

Inspired by [Czkawka](https://github.com/qarmin/czkawka), **Dedupus** re-imagines duplicate file finding and disk cleaning with a modern tech stack and streamlined user experience.

---

## ⚡ Core Philosophy & Modern Tech Stack

Dedupus decouples raw high-throughput scanning logic from UI presentation:

- **Core Engine (`dedupus_core`)**: Pure Rust
  - **Multi-threaded I/O & CPU-bound hashing**: Powered by `rayon` & memory-mapped I/O (`memmap2`).
  - **Hashing algorithms**: BLAKE3 and XXH3 (64-bit / 128-bit) for ultra-fast exact duplicate matching.
  - **Multi-stage filtering**: Size grouping ➔ Header/partial hash ➔ Full hash (minimizes disk reads).
  - **Media analysis**: Perceptual hashing for similar images, acoustic fingerprinting for audio, frame extraction for videos.
- **GUI Frontend (`Dedupus` / Tauri v2)**:
  - **Tauri v2**: Lightweight native OS webview (WebView2 on Windows) with tiny binary footprint (~15MB vs Electron's 150MB+).
  - **Modern UI**: React 19 + TypeScript + a custom CSS design system with light/dark themes and a frameless window.
  - **High-volume virtualization**: Handles hundreds of thousands of scan results smoothly using `@tanstack/react-virtual`.
- **CLI (`dedupus_cli`)**: High-performance command-line utility for servers and automated scripts.

---

## 🚀 Key Features

- [x] **Exact Duplicates**: Multi-stage detection (Size ➔ 4KB prefix hash ➔ Full BLAKE3/XXH3 hash).
- [x] **Similar Images**: Perceptual hashing (dHash, aHash) with configurable similarity thresholds.
- [x] **Similar Audio & Video**: Acoustic energy-envelope fingerprints for audio; keyframe dHash sampling for video (requires `ffmpeg`/`ffprobe` on PATH for video frames).
- [x] **Disk Hygiene**:
  - [x] Empty files & empty directory trees.
  - [x] Temporary & cache files.
  - [x] Broken symlinks.
  - [x] Invalid Windows shortcuts (.lnk).
  - [x] Broken media files (corrupted images, unreadable archives, invalid audio).
  - [x] Bad file extensions (content signature does not match the file name).
  - [x] Big files analyzer.
- [x] **Safe Cleaning Actions**: Move to Recycle Bin / Trash, hardlinking, or permanent deletion.
- [x] **Scan Cache**: Persist file hashes across scans to dramatically speed up subsequent runs.
- [x] **Live Scan Feedback**: Real-time stage, running file count and hashing progress, with instant cancellation.
- [x] **Large Result Lists**: Virtualized rendering (TanStack Virtual) keeps scans with 100,000+ rows responsive.
- [x] **File Actions**: Right-click any result to open it, reveal it in Explorer, copy its path, move it to the Recycle Bin, delete it permanently, or replace duplicates with hard links.

---

## 📦 Build Installers

Dedupus ships as a standalone Windows desktop app with NSIS and MSI installers.

### Runtime requirements
- Windows 10/11 with Microsoft Edge WebView2 (preinstalled on Windows 11 and current Windows 10)
- `ffmpeg`/`ffprobe` on `PATH` is optional; it enables perceptual frame analysis for similar-video scans

### Prerequisites
- [Rust](https://rustup.rs/) (>= 1.80)
- [Node.js](https://nodejs.org/) (>= 20) & [pnpm](https://pnpm.io/)

### Build
```bash
pnpm install
pnpm tauri build
```

Artifacts are written to:
- `target/release/bundle/nsis/Dedupus_0.1.0_x64-setup.exe`
- `target/release/bundle/msi/Dedupus_0.1.0_x64_en-US.msi`

The installers are currently unsigned, so Windows SmartScreen may show an "Unknown publisher" warning on first launch. Build from source if you prefer to verify the binary yourself.

---

## 🛠️ Repository Setup & Workflow

### Initial Setup
```bash
# Clone the repository
git clone https://github.com/BerryUIKI/Dedupus.git
cd Dedupus

# Install frontend dependencies
pnpm install

# Build core Rust engine
cargo build --release
```

---

## 📄 License

MIT OR Apache-2.0 — see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
