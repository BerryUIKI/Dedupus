# Changelog

## [0.1.0] - 2026-11-06

First public release of Dedupus.

### Scanners

- **Exact duplicates** with multi-stage filtering (size, 4 KB prefix hash, full BLAKE3 / XXH3 hash).
- **Similar images** using dHash / aHash perceptual hashing with configurable tolerance.
- **Similar audio** using 128-bit acoustic energy-envelope fingerprints.
- **Similar video** using keyframes sampled at 10%, 50% and 80% of the runtime (requires `ffmpeg`/`ffprobe`).
- **Empty files** and **empty folders**.
- **Temporary files** detected by name and extension.
- **Broken files**: corrupted images, archives/PDFs, unreadable audio, broken symlinks and invalid `.lnk` shortcuts.
- **Bad extensions**: content signature does not match the file extension.
- **Big files** ranked by size.

### Safe cleaning

- Move to the Windows Recycle Bin / system Trash.
- Permanent deletion with a native confirmation dialog.
- Replace duplicate files with hard links from the result context menu.

### Desktop app

- Frameless Tauri v2 window with a custom title bar and light/dark themes.
- Live scan progress (stage, running file count, hashing progress) with cancellation.
- Virtualized result lists for scans with 100,000+ rows.
- Right-click actions: open, reveal in Explorer, copy path, Recycle Bin, permanent delete and hardlink replacement.
- Persistent scan cache for hashes and perceptual image hashes.
- Native folder picker.

### Packaging

- Windows NSIS and MSI installers built with `pnpm tauri build`.
- Dual-licensed under MIT OR Apache-2.0.
