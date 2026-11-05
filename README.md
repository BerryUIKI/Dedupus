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
- **GUI Frontend (`dedupus_gui` / Tauri v2)**:
  - **Tauri v2**: Lightweight native OS webview (WebView2 on Windows) with tiny binary footprint (~15MB vs Electron's 150MB+).
  - **Modern UI**: React 19 + TypeScript + Tailwind CSS + Radix/shadcn primitives.
  - **High-volume virtualization**: Handles hundreds of thousands of scan results smoothly using `@tanstack/react-virtual`.
- **CLI (`dedupus_cli`)**: High-performance command-line utility for servers and automated scripts.

---

## 🚀 Key Features Planned

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

## 🛠️ Repository Setup & Workflow

### Prerequisites
- [Rust](https://rustup.rs/) (>= 1.80)
- [Node.js](https://nodejs.org/) (>= 20) & [pnpm](https://pnpm.io/)

### Initial Setup
```bash
# Clone the repository
git clone https://github.com/BerryUIKI/Dedupus.git
cd Dedupus

# Install frontend dependencies (when UI is initialized)
pnpm install

# Build core Rust engine
cargo build --release
```

---

## 📄 License

MIT OR Apache-2.0
