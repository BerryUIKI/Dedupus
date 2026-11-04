import { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { LucideIcon } from "lucide-react";
import {
  FolderSearch,
  Files,
  Images,
  Music,
  Video,
  FolderMinus,
  Trash2,
  Play,
  RotateCw,
  HardDrive,
  Copy,
  FolderOpen,
  Sparkles,
  CheckSquare,
  Square,
  Zap,
  ArrowDownWideNarrow,
  Eraser,
  AlertTriangle,
  Minus,
  Maximize2,
  Minimize2,
  X,
} from "lucide-react";
import "./App.css";

interface ScannedFile {
  path: string;
  size: number;
  modified_secs: number | null;
}

interface TempFileEntry {
  path: string;
  size: number;
  modified_secs: number | null;
  reason: string;
}

interface BrokenFileEntry {
  path: string;
  size: number;
  modified_secs: number | null;
  reason: string;
}

interface DuplicateGroup {
  file_size: number;
  hash: string;
  files: ScannedFile[];
}

interface ScannedImage {
  path: string;
  size: number;
  width: number;
  height: number;
  hash: number;
}

interface SimilarImageGroup {
  similarity_level: string;
  images: ScannedImage[];
}

interface ScannedAudio {
  path: string;
  size: number;
  duration_secs: number;
  sample_rate: number;
  channels: number;
  fingerprint: [number, number];
}

interface SimilarAudioGroup {
  similarity_info: string;
  tracks: ScannedAudio[];
}

interface ScannedVideo {
  path: string;
  size: number;
  duration_secs: number | null;
  width: number | null;
  height: number | null;
  frame_hashes: number[];
}

interface SimilarVideoGroup {
  similarity_info: string;
  videos: ScannedVideo[];
}

interface CleanReport {
  successful: string[];
  failed: [string, string][];
  bytes_freed: number;
}

type Mode =
  | "duplicates"
  | "similar_images"
  | "similar_audio"
  | "similar_video"
  | "big_files"
  | "temp_files"
  | "broken_files"
  | "empty_files"
  | "empty_folders";

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`;
}

const MODE_INFO: Record<Mode, { title: string; subtitle: string }> = {
  duplicates: {
    title: "Exact Duplicates",
    subtitle:
      "Byte-identical files detected through multi-stage hashing (size → 4 KB prefix → full BLAKE3/XXH3).",
  },
  similar_images: {
    title: "Similar Images",
    subtitle:
      "Visually similar pictures grouped with perceptual hashing (dHash / aHash), best resolution first.",
  },
  similar_audio: {
    title: "Similar Audio",
    subtitle:
      "Tracks clustered by duration and 128-bit acoustic energy-envelope fingerprints.",
  },
  similar_video: {
    title: "Similar Video",
    subtitle:
      "Clips compared using keyframes sampled at 10%, 50% and 80% of the runtime.",
  },
  big_files: {
    title: "Big Files",
    subtitle: "The largest files in the selected tree, ranked by size.",
  },
  temp_files: {
    title: "Temporary Files",
    subtitle:
      "Temp, backup, partial-download and OS junk files identified by name and extension.",
  },
  broken_files: {
    title: "Broken Files",
    subtitle:
      "Images, archives, PDFs and audio validated for corruption; broken symlinks included.",
  },
  empty_files: {
    title: "Empty Files",
    subtitle: "Zero-byte files that are usually safe to remove.",
  },
  empty_folders: {
    title: "Empty Folders",
    subtitle: "Directories with no files left inside, scanned bottom-up.",
  },
};

const NAV_GROUPS: {
  label: string;
  items: { mode: Mode; label: string; icon: LucideIcon }[];
}[] = [
  {
    label: "Find duplicates",
    items: [
      { mode: "duplicates", label: "Exact Duplicates", icon: Files },
      { mode: "similar_images", label: "Similar Images", icon: Images },
      { mode: "similar_audio", label: "Similar Audio", icon: Music },
      { mode: "similar_video", label: "Similar Video", icon: Video },
    ],
  },
  {
    label: "Analyze",
    items: [{ mode: "big_files", label: "Big Files", icon: ArrowDownWideNarrow }],
  },
  {
    label: "Clean up",
    items: [
      { mode: "temp_files", label: "Temp Files", icon: Eraser },
      { mode: "broken_files", label: "Broken Files", icon: AlertTriangle },
      { mode: "empty_files", label: "Empty Files", icon: Trash2 },
      { mode: "empty_folders", label: "Empty Folders", icon: FolderMinus },
    ],
  },
];

export default function App() {
  const [mode, setMode] = useState<Mode>("duplicates");
  const [scanPath, setScanPath] = useState("");
  const [minSize, setMinSize] = useState<number>(1024); // 1KB default
  const [algo, setAlgo] = useState("blake3");
  const [imageAlgo, setImageAlgo] = useState("gradient");
  const [hammingDistance, setHammingDistance] = useState<number>(4);
  const [durationDiff, setDurationDiff] = useState<number>(2.0);
  const [bigMinMb, setBigMinMb] = useState<number>(100);
  const [bigLimit, setBigLimit] = useState<number>(100);
  const [tempMinKb, setTempMinKb] = useState<number>(0);
  const [useCache, setUseCache] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [statusMsg, setStatusMsg] = useState("");

  const [duplicates, setDuplicates] = useState<DuplicateGroup[]>([]);
  const [similarImages, setSimilarImages] = useState<SimilarImageGroup[]>([]);
  const [similarAudio, setSimilarAudio] = useState<SimilarAudioGroup[]>([]);
  const [similarVideos, setSimilarVideos] = useState<SimilarVideoGroup[]>([]);
  const [bigFiles, setBigFiles] = useState<ScannedFile[]>([]);
  const [tempFiles, setTempFiles] = useState<TempFileEntry[]>([]);
  const [brokenFiles, setBrokenFiles] = useState<BrokenFileEntry[]>([]);
  const [emptyFiles, setEmptyFiles] = useState<ScannedFile[]>([]);
  const [emptyFolders, setEmptyFolders] = useState<string[]>([]);

  // Set of selected file paths for actions
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const appWindow = getCurrentWindow();
        const maximized = await appWindow.isMaximized();
        if (active) setIsMaximized(maximized);

        unlisten = await appWindow.onResized(async () => {
          try {
            const state = await appWindow.isMaximized();
            if (active) setIsMaximized(state);
          } catch {
            // ignore
          }
        });
      } catch {
        // Browser preview without the Tauri runtime
      }
    })();

    return () => {
      active = false;
      if (unlisten) unlisten();
    };
  }, []);

  const windowAction = async (action: "minimize" | "maximize" | "close") => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const appWindow = getCurrentWindow();
      if (action === "minimize") {
        await appWindow.minimize();
      } else if (action === "maximize") {
        await appWindow.toggleMaximize();
      } else {
        await appWindow.close();
      }
    } catch {
      // Browser preview: window controls are inert
    }
  };

  const handleBrowse = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Select a folder to scan",
      });
      if (typeof selected === "string") {
        setScanPath(selected);
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Folder selection failed: ${msg}`);
    }
  };

  const handleScan = async () => {
    if (!scanPath.trim()) {
      setStatusMsg("Please specify a directory path to scan.");
      return;
    }

    setScanning(true);
    setStatusMsg("Scanning in progress...");
    setSelectedPaths(new Set());

    try {
      if (mode === "duplicates") {
        const res = await invoke<DuplicateGroup[]>("scan_duplicates", {
          directories: [scanPath.trim()],
          exclude: [],
          minSize: minSize > 0 ? minSize : null,
          maxSize: null,
          algo,
          useCache,
        });
        setDuplicates(res);
        const totalWasted = res.reduce(
          (acc, g) => acc + g.file_size * (g.files.length - 1),
          0
        );
        setStatusMsg(
          `Scan completed: Found ${res.length} duplicate groups (${formatBytes(
            totalWasted
          )} reclaimable)`
        );
      } else if (mode === "similar_images") {
        const res = await invoke<SimilarImageGroup[]>("scan_similar_images", {
          directories: [scanPath.trim()],
          exclude: [],
          distance: hammingDistance,
          algo: imageAlgo,
          useCache,
        });
        setSimilarImages(res);
        const totalImages = res.reduce((acc, g) => acc + g.images.length, 0);
        setStatusMsg(
          `Scan completed: Found ${res.length} similar image groups (${totalImages} total images)`
        );
      } else if (mode === "similar_audio") {
        const res = await invoke<SimilarAudioGroup[]>("scan_similar_audio", {
          directories: [scanPath.trim()],
          exclude: [],
          maxDurationDiff: durationDiff,
        });
        setSimilarAudio(res);
        const totalTracks = res.reduce((acc, g) => acc + g.tracks.length, 0);
        setStatusMsg(
          `Scan completed: Found ${res.length} similar audio groups (${totalTracks} total tracks)`
        );
      } else if (mode === "similar_video") {
        const res = await invoke<SimilarVideoGroup[]>("scan_similar_videos", {
          directories: [scanPath.trim()],
          exclude: [],
          maxDurationDiff: durationDiff,
        });
        setSimilarVideos(res);
        const totalVideos = res.reduce((acc, g) => acc + g.videos.length, 0);
        setStatusMsg(
          `Scan completed: Found ${res.length} similar video groups (${totalVideos} total videos)`
        );
      } else if (mode === "big_files") {
        const res = await invoke<ScannedFile[]>("scan_big_files", {
          directories: [scanPath.trim()],
          exclude: [],
          minSize: Math.max(1, Math.round(bigMinMb * 1024 * 1024)),
          maxResults: Math.max(1, bigLimit),
        });
        setBigFiles(res);
        const totalBytes = res.reduce((acc, f) => acc + f.size, 0);
        setStatusMsg(
          `Scan completed: Found ${res.length} large files (${formatBytes(
            totalBytes
          )} total)`
        );
      } else if (mode === "temp_files") {
        const res = await invoke<TempFileEntry[]>("scan_temp_files", {
          directories: [scanPath.trim()],
          exclude: [],
          minSize: Math.max(0, Math.round(tempMinKb * 1024)),
        });
        setTempFiles(res);
        const totalBytes = res.reduce((acc, f) => acc + f.size, 0);
        setStatusMsg(
          `Scan completed: Found ${res.length} temporary files (${formatBytes(
            totalBytes
          )} total)`
        );
      } else if (mode === "broken_files") {
        const res = await invoke<BrokenFileEntry[]>("scan_broken_files", {
          directories: [scanPath.trim()],
          exclude: [],
        });
        setBrokenFiles(res);
        setStatusMsg(
          `Scan completed: Found ${res.length} broken files`
        );
      } else if (mode === "empty_files") {
        const res = await invoke<ScannedFile[]>("scan_empty_files", {
          directories: [scanPath.trim()],
          exclude: [],
        });
        setEmptyFiles(res);
        setStatusMsg(`Scan completed: Found ${res.length} empty files`);
      } else if (mode === "empty_folders") {
        const res = await invoke<string[]>("scan_empty_folders", {
          directories: [scanPath.trim()],
          exclude: [],
        });
        setEmptyFolders(res);
        setStatusMsg(`Scan completed: Found ${res.length} empty folders`);
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Scan failed: ${msg}`);
    } finally {
      setScanning(false);
    }
  };

  // Toggle single item selection
  const toggleSelect = (path: string) => {
    setSelectedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  // Select all except the first/best in each group
  const selectDuplicatesKeepFirst = () => {
    const next = new Set<string>();
    if (mode === "duplicates") {
      for (const group of duplicates) {
        for (let i = 1; i < group.files.length; i++) {
          next.add(group.files[i].path);
        }
      }
    } else if (mode === "similar_images") {
      for (const group of similarImages) {
        for (let i = 1; i < group.images.length; i++) {
          next.add(group.images[i].path);
        }
      }
    } else if (mode === "similar_audio") {
      for (const group of similarAudio) {
        for (let i = 1; i < group.tracks.length; i++) {
          next.add(group.tracks[i].path);
        }
      }
    } else if (mode === "similar_video") {
      for (const group of similarVideos) {
        for (let i = 1; i < group.videos.length; i++) {
          next.add(group.videos[i].path);
        }
      }
    } else if (mode === "big_files") {
      for (const file of bigFiles) {
        next.add(file.path);
      }
    } else if (mode === "temp_files") {
      for (const file of tempFiles) {
        next.add(file.path);
      }
    } else if (mode === "broken_files") {
      for (const file of brokenFiles) {
        next.add(file.path);
      }
    } else if (mode === "empty_files") {
      for (const file of emptyFiles) {
        next.add(file.path);
      }
    } else if (mode === "empty_folders") {
      for (const folder of emptyFolders) {
        next.add(folder);
      }
    }
    setSelectedPaths(next);
  };

  const clearSelection = () => {
    setSelectedPaths(new Set());
  };

  // Calculate size of selected files
  const selectedBytes = useMemo(() => {
    let bytes = 0;
    if (mode === "duplicates") {
      for (const group of duplicates) {
        for (const file of group.files) {
          if (selectedPaths.has(file.path)) {
            bytes += file.size;
          }
        }
      }
    } else if (mode === "similar_images") {
      for (const group of similarImages) {
        for (const img of group.images) {
          if (selectedPaths.has(img.path)) {
            bytes += img.size;
          }
        }
      }
    } else if (mode === "similar_audio") {
      for (const group of similarAudio) {
        for (const track of group.tracks) {
          if (selectedPaths.has(track.path)) {
            bytes += track.size;
          }
        }
      }
    } else if (mode === "similar_video") {
      for (const group of similarVideos) {
        for (const vid of group.videos) {
          if (selectedPaths.has(vid.path)) {
            bytes += vid.size;
          }
        }
      }
    } else if (mode === "big_files") {
      for (const file of bigFiles) {
        if (selectedPaths.has(file.path)) {
          bytes += file.size;
        }
      }
    } else if (mode === "temp_files") {
      for (const file of tempFiles) {
        if (selectedPaths.has(file.path)) {
          bytes += file.size;
        }
      }
    } else if (mode === "broken_files") {
      for (const file of brokenFiles) {
        if (selectedPaths.has(file.path)) {
          bytes += file.size;
        }
      }
    }
    return bytes;
  }, [
    selectedPaths,
    duplicates,
    similarImages,
    similarAudio,
    similarVideos,
    bigFiles,
    tempFiles,
    brokenFiles,
    mode,
  ]);

  // Trash action
  const handleMoveToTrash = async () => {
    if (selectedPaths.size === 0) return;
    const pathsToTrash = Array.from(selectedPaths);
    setCleaning(true);

    try {
      const report = await invoke<CleanReport>("delete_to_trash", {
        paths: pathsToTrash,
      });

      const succSet = new Set(report.successful);

      // Remove trashed files from state
      if (mode === "duplicates") {
        setDuplicates((prev) =>
          prev
            .map((g) => ({
              ...g,
              files: g.files.filter((f) => !succSet.has(f.path)),
            }))
            .filter((g) => g.files.length >= 2)
        );
      } else if (mode === "similar_images") {
        setSimilarImages((prev) =>
          prev
            .map((g) => ({
              ...g,
              images: g.images.filter((img) => !succSet.has(img.path)),
            }))
            .filter((g) => g.images.length >= 2)
        );
      } else if (mode === "similar_audio") {
        setSimilarAudio((prev) =>
          prev
            .map((g) => ({
              ...g,
              tracks: g.tracks.filter((t) => !succSet.has(t.path)),
            }))
            .filter((g) => g.tracks.length >= 2)
        );
      } else if (mode === "similar_video") {
        setSimilarVideos((prev) =>
          prev
            .map((g) => ({
              ...g,
              videos: g.videos.filter((v) => !succSet.has(v.path)),
            }))
            .filter((g) => g.videos.length >= 2)
        );
      } else if (mode === "empty_files") {
        setEmptyFiles((prev) => prev.filter((f) => !succSet.has(f.path)));
      } else if (mode === "empty_folders") {
        setEmptyFolders((prev) => prev.filter((p) => !succSet.has(p)));
      } else if (mode === "big_files") {
        setBigFiles((prev) => prev.filter((f) => !succSet.has(f.path)));
      } else if (mode === "temp_files") {
        setTempFiles((prev) => prev.filter((f) => !succSet.has(f.path)));
      } else if (mode === "broken_files") {
        setBrokenFiles((prev) => prev.filter((f) => !succSet.has(f.path)));
      }

      setSelectedPaths(new Set());
      setStatusMsg(
        `🗑️ Moved ${report.successful.length} items to Recycle Bin (${formatBytes(
          report.bytes_freed
        )} freed).`
      );

      if (report.failed.length > 0) {
        setStatusMsg(
          (prev) => `${prev} Failed to trash ${report.failed.length} items.`
        );
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Trash action failed: ${msg}`);
    } finally {
      setCleaning(false);
    }
  };

  const handleClearCache = async () => {
    try {
      await invoke("clear_cache");
      setStatusMsg("✨ Scan cache cleared.");
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Clear cache failed: ${msg}`);
    }
  };

  return (
    <div className="app-frame">
      {/* Custom window title bar */}
      <div className="window-bar" data-tauri-drag-region>
        <div className="window-bar-title" data-tauri-drag-region>
          <FolderSearch size={14} />
          <span>Dedupus</span>
        </div>
        <div className="window-controls">
          <button
            className="window-btn"
            onClick={() => windowAction("minimize")}
            title="Minimize"
            aria-label="Minimize"
          >
            <Minus size={15} />
          </button>
          <button
            className="window-btn"
            onClick={() => windowAction("maximize")}
            title={isMaximized ? "Restore" : "Maximize"}
            aria-label={isMaximized ? "Restore" : "Maximize"}
          >
            {isMaximized ? <Minimize2 size={14} /> : <Maximize2 size={13} />}
          </button>
          <button
            className="window-btn close"
            onClick={() => windowAction("close")}
            title="Close"
            aria-label="Close"
          >
            <X size={15} />
          </button>
        </div>
      </div>

      <div className="app-shell">
      {/* Sidebar navigation */}
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-icon">
            <FolderSearch size={22} />
          </div>
          <div>
            <h1>Dedupus</h1>
            <p>Disk Cleaner</p>
          </div>
        </div>

        <nav className="nav-groups">
          {NAV_GROUPS.map((group) => (
            <div className="nav-group" key={group.label}>
              <div className="nav-group-label">{group.label}</div>
              {group.items.map((item) => {
                const Icon = item.icon;
                return (
                  <button
                    key={item.mode}
                    className={`nav-item ${
                      mode === item.mode ? "active" : ""
                    }`}
                    onClick={() => {
                      setMode(item.mode);
                      setSelectedPaths(new Set());
                    }}
                  >
                    <Icon size={16} />
                    <span>{item.label}</span>
                  </button>
                );
              })}
            </div>
          ))}
        </nav>

        <div className="sidebar-footer">v0.1.0 · Tauri + React</div>
      </aside>

      {/* Main area */}
      <div className="main-area">
        <header className="topbar">
          <div className="topbar-title">
            <h1>{MODE_INFO[mode].title}</h1>
            <p>{MODE_INFO[mode].subtitle}</p>
          </div>

          <div className="cache-group">
            <label
              className="cache-toggle"
              title="Enable persistent scan cache to accelerate subsequent scans"
            >
              <input
                type="checkbox"
                checked={useCache}
                onChange={(e) => setUseCache(e.target.checked)}
                disabled={scanning || cleaning}
              />
              <Zap size={14} className={useCache ? "zap-active" : ""} />
              <span>Cache</span>
            </label>
            <button
              className="clear-cache-btn"
              title="Clear all saved scan cache"
              onClick={handleClearCache}
              disabled={scanning || cleaning}
            >
              Clear
            </button>
          </div>
        </header>

      {/* Control Bar */}
      <section className="control-bar">
        <div className="input-group flex-1 path-group">
          <FolderOpen size={18} className="input-icon" />
          <input
            type="text"
            placeholder="Enter a folder path, or browse…"
            value={scanPath}
            onChange={(e) => setScanPath(e.target.value)}
            disabled={scanning || cleaning}
          />
          <button
            className="browse-btn"
            onClick={handleBrowse}
            disabled={scanning || cleaning}
            title="Choose a folder to scan"
          >
            <FolderOpen size={15} />
            <span>Browse</span>
          </button>
        </div>

        {mode === "duplicates" && (
          <>
            <div className="input-group">
              <label>Min Size (B):</label>
              <input
                type="number"
                value={minSize}
                onChange={(e) => setMinSize(Number(e.target.value))}
                style={{ width: "90px" }}
                disabled={scanning || cleaning}
              />
            </div>
            <div className="input-group">
              <label>Algorithm:</label>
              <select
                value={algo}
                onChange={(e) => setAlgo(e.target.value)}
                disabled={scanning || cleaning}
              >
                <option value="blake3">BLAKE3 (Default)</option>
                <option value="xxh3_128">XXH3-128</option>
                <option value="xxh3_64">XXH3-64</option>
              </select>
            </div>
          </>
        )}

        {mode === "similar_images" && (
          <>
            <div className="input-group">
              <label>Tolerance:</label>
              <select
                value={hammingDistance}
                onChange={(e) => setHammingDistance(Number(e.target.value))}
                disabled={scanning || cleaning}
              >
                <option value={0}>0 - Identical</option>
                <option value={2}>2 - Very High Similarity</option>
                <option value={4}>4 - High Similarity (Default)</option>
                <option value={8}>8 - Medium Similarity</option>
                <option value={12}>12 - Low Similarity</option>
              </select>
            </div>
            <div className="input-group">
              <label>Method:</label>
              <select
                value={imageAlgo}
                onChange={(e) => setImageAlgo(e.target.value)}
                disabled={scanning || cleaning}
              >
                <option value="gradient">Gradient (dHash)</option>
                <option value="average">Average (aHash)</option>
              </select>
            </div>
          </>
        )}

        {(mode === "similar_audio" || mode === "similar_video") && (
          <div className="input-group">
            <label>Max Duration Diff (s):</label>
            <input
              type="number"
              step="0.5"
              value={durationDiff}
              onChange={(e) => setDurationDiff(Number(e.target.value))}
              style={{ width: "80px" }}
              disabled={scanning || cleaning}
            />
          </div>
        )}

        {mode === "similar_video" && (
          <span
            className="ffmpeg-hint"
            title="Video frame analysis uses ffmpeg/ffprobe from PATH; without them videos are grouped by size and duration only"
          >
            ffmpeg for frames
          </span>
        )}

        {mode === "big_files" && (
          <>
            <div className="input-group">
              <label>Min Size (MB):</label>
              <input
                type="number"
                min={1}
                value={bigMinMb}
                onChange={(e) => setBigMinMb(Number(e.target.value))}
                style={{ width: "80px" }}
                disabled={scanning || cleaning}
              />
            </div>
            <div className="input-group">
              <label>Top N:</label>
              <input
                type="number"
                min={1}
                value={bigLimit}
                onChange={(e) => setBigLimit(Number(e.target.value))}
                style={{ width: "70px" }}
                disabled={scanning || cleaning}
              />
            </div>
          </>
        )}

        {mode === "temp_files" && (
          <div className="input-group">
            <label>Min Size (KB):</label>
            <input
              type="number"
              min={0}
              value={tempMinKb}
              onChange={(e) => setTempMinKb(Number(e.target.value))}
              style={{ width: "80px" }}
              disabled={scanning || cleaning}
            />
          </div>
        )}

        <button
          className="scan-btn"
          onClick={handleScan}
          disabled={scanning || cleaning}
        >
          {scanning ? (
            <>
              <RotateCw size={16} className="spin" />
              <span>Scanning...</span>
            </>
          ) : (
            <>
              <Play size={16} />
              <span>Start Scan</span>
            </>
          )}
        </button>
      </section>

      {/* Indeterminate progress while a scan is running */}
      {scanning && (
        <div className="progress-track">
          <div className="progress-bar" />
        </div>
      )}

      {/* Sub-bar: Selection Helpers when results exist */}
      {((mode === "duplicates" && duplicates.length > 0) ||
        (mode === "similar_images" && similarImages.length > 0) ||
        (mode === "similar_audio" && similarAudio.length > 0) ||
        (mode === "similar_video" && similarVideos.length > 0) ||
        (mode === "big_files" && bigFiles.length > 0) ||
        (mode === "temp_files" && tempFiles.length > 0) ||
        (mode === "broken_files" && brokenFiles.length > 0) ||
        (mode === "empty_files" && emptyFiles.length > 0) ||
        (mode === "empty_folders" && emptyFolders.length > 0)) && (
        <div className="selection-toolbar">
          <div className="selection-actions">
            <button
              className="tool-btn"
              onClick={selectDuplicatesKeepFirst}
              disabled={cleaning}
            >
              <CheckSquare size={14} />
              <span>
                {mode === "big_files" ||
                mode === "temp_files" ||
                mode === "broken_files"
                  ? "Select All Files"
                  : "Select All Duplicates (Keep 1st)"}
              </span>
            </button>
            <button
              className="tool-btn"
              onClick={clearSelection}
              disabled={selectedPaths.size === 0 || cleaning}
            >
              <Square size={14} />
              <span>Deselect All</span>
            </button>
          </div>
          <span className="selected-count">
            {selectedPaths.size} items selected
            {selectedBytes > 0 && ` (${formatBytes(selectedBytes)} reclaimable)`}
          </span>
        </div>
      )}

      {/* Status Bar */}
      {statusMsg && <div className="status-banner">{statusMsg}</div>}

      {/* Main Content Area */}
      <main
        className={`results-container ${
          selectedPaths.size > 0 ? "with-footer" : ""
        }`}
      >
        {mode === "duplicates" && (
          <div className="list-view">
            {duplicates.length === 0 ? (
              <div className="empty-state">
                <HardDrive size={48} opacity={0.3} />
                <p>No duplicates found or no scan executed yet.</p>
              </div>
            ) : (
              duplicates.map((group, gIdx) => {
                const wasted = group.file_size * (group.files.length - 1);
                return (
                  <div key={gIdx} className="result-card">
                    <div className="card-header">
                      <span className="badge">Group #{gIdx + 1}</span>
                      <span className="meta">
                        {formatBytes(group.file_size)} / file
                      </span>
                      <span className="wasted-badge">
                        Wasted: {formatBytes(wasted)}
                      </span>
                      <span className="hash-code" title={group.hash}>
                        Hash: {group.hash.substring(0, 16)}...
                      </span>
                    </div>
                    <ul className="file-list">
                      {group.files.map((file, fIdx) => {
                        const isSelected = selectedPaths.has(file.path);
                        const isFirst = fIdx === 0;
                        return (
                          <li
                            key={fIdx}
                            className={`file-item ${
                              isSelected ? "selected-item" : ""
                            }`}
                          >
                            <label className="checkbox-label">
                              <input
                                type="checkbox"
                                checked={isSelected}
                                onChange={() => toggleSelect(file.path)}
                              />
                              {isFirst && (
                                <span className="keeper-badge">Original</span>
                              )}
                              <span className="file-path">{file.path}</span>
                            </label>
                            <div className="item-actions">
                              <button
                                className="icon-action-btn"
                                title="Copy path"
                                onClick={() =>
                                  navigator.clipboard.writeText(file.path)
                                }
                              >
                                <Copy size={14} />
                              </button>
                            </div>
                          </li>
                        );
                      })}
                    </ul>
                  </div>
                );
              })
            )}
          </div>
        )}

        {mode === "similar_images" && (
          <div className="list-view">
            {similarImages.length === 0 ? (
              <div className="empty-state">
                <Images size={48} opacity={0.3} />
                <p>No similar images found or scan not started.</p>
              </div>
            ) : (
              similarImages.map((group, gIdx) => (
                <div key={gIdx} className="result-card">
                  <div className="card-header">
                    <span className="badge purple-badge">
                      Image Group #{gIdx + 1}
                    </span>
                    <span className="meta">
                      {group.images.length} candidates
                    </span>
                    <span className="tolerance-tag">
                      {group.similarity_level}
                    </span>
                  </div>
                  <ul className="file-list">
                    {group.images.map((img, iIdx) => {
                      const isHighestRes = iIdx === 0;
                      const isSelected = selectedPaths.has(img.path);
                      return (
                        <li
                          key={iIdx}
                          className={`file-item ${
                            isSelected ? "selected-item" : ""
                          }`}
                        >
                          <label className="checkbox-label">
                            <input
                              type="checkbox"
                              checked={isSelected}
                              onChange={() => toggleSelect(img.path)}
                            />
                            {isHighestRes && (
                              <span
                                className="best-tag"
                                title="Highest resolution in group"
                              >
                                <Sparkles size={12} /> Best Res
                              </span>
                            )}
                            <span className="file-path">{img.path}</span>
                          </label>
                          <div className="image-meta">
                            <span className="res-badge">
                              {img.width} × {img.height}
                            </span>
                            <span className="size-badge">
                              {formatBytes(img.size)}
                            </span>
                            <button
                              className="icon-action-btn"
                              title="Copy path"
                              onClick={() =>
                                navigator.clipboard.writeText(img.path)
                              }
                            >
                              <Copy size={14} />
                            </button>
                          </div>
                        </li>
                      );
                    })}
                  </ul>
                </div>
              ))
            )}
          </div>
        )}

        {mode === "similar_audio" && (
          <div className="list-view">
            {similarAudio.length === 0 ? (
              <div className="empty-state">
                <Music size={48} opacity={0.3} />
                <p>No similar audio tracks found or scan not started.</p>
              </div>
            ) : (
              similarAudio.map((group, gIdx) => (
                <div key={gIdx} className="result-card">
                  <div className="card-header">
                    <span className="badge teal-badge">
                      Audio Group #{gIdx + 1}
                    </span>
                    <span className="meta">
                      {group.tracks.length} tracks
                    </span>
                    <span className="tolerance-tag">
                      {group.similarity_info}
                    </span>
                  </div>
                  <ul className="file-list">
                    {group.tracks.map((track, tIdx) => {
                      const isSelected = selectedPaths.has(track.path);
                      const isFirst = tIdx === 0;
                      return (
                        <li
                          key={tIdx}
                          className={`file-item ${
                            isSelected ? "selected-item" : ""
                          }`}
                        >
                          <label className="checkbox-label">
                            <input
                              type="checkbox"
                              checked={isSelected}
                              onChange={() => toggleSelect(track.path)}
                            />
                            {isFirst && (
                              <span className="keeper-badge">Original</span>
                            )}
                            <span className="file-path">{track.path}</span>
                          </label>
                          <div className="image-meta">
                            <span className="res-badge">
                              {track.duration_secs.toFixed(1)}s
                            </span>
                            <span className="res-badge">
                              {track.sample_rate}Hz ({track.channels}ch)
                            </span>
                            <span className="size-badge">
                              {formatBytes(track.size)}
                            </span>
                            <button
                              className="icon-action-btn"
                              title="Copy path"
                              onClick={() =>
                                navigator.clipboard.writeText(track.path)
                              }
                            >
                              <Copy size={14} />
                            </button>
                          </div>
                        </li>
                      );
                    })}
                  </ul>
                </div>
              ))
            )}
          </div>
        )}

        {mode === "similar_video" && (
          <div className="list-view">
            {similarVideos.length === 0 ? (
              <div className="empty-state">
                <Video size={48} opacity={0.3} />
                <p>No similar videos found or scan not started.</p>
              </div>
            ) : (
              similarVideos.map((group, gIdx) => (
                <div key={gIdx} className="result-card">
                  <div className="card-header">
                    <span className="badge amber-badge">
                      Video Group #{gIdx + 1}
                    </span>
                    <span className="meta">
                      {group.videos.length} videos
                    </span>
                    <span className="tolerance-tag">
                      {group.similarity_info}
                    </span>
                  </div>
                  <ul className="file-list">
                    {group.videos.map((vid, vIdx) => {
                      const isSelected = selectedPaths.has(vid.path);
                      const isFirst = vIdx === 0;
                      return (
                        <li
                          key={vIdx}
                          className={`file-item ${
                            isSelected ? "selected-item" : ""
                          }`}
                        >
                          <label className="checkbox-label">
                            <input
                              type="checkbox"
                              checked={isSelected}
                              onChange={() => toggleSelect(vid.path)}
                            />
                            {isFirst && (
                              <span className="best-tag">
                                <Sparkles size={12} /> Best
                              </span>
                            )}
                            <span className="file-path">{vid.path}</span>
                          </label>
                          <div className="image-meta">
                            {vid.width && vid.height && (
                              <span className="res-badge">
                                {vid.width} × {vid.height}
                              </span>
                            )}
                            <span className="size-badge">
                              {formatBytes(vid.size)}
                            </span>
                            <button
                              className="icon-action-btn"
                              title="Copy path"
                              onClick={() =>
                                navigator.clipboard.writeText(vid.path)
                              }
                            >
                              <Copy size={14} />
                            </button>
                          </div>
                        </li>
                      );
                    })}
                  </ul>
                </div>
              ))
            )}
          </div>
        )}

        {mode === "big_files" && (
          <div className="list-view">
            {bigFiles.length === 0 ? (
              <div className="empty-state">
                <ArrowDownWideNarrow size={48} opacity={0.3} />
                <p>
                  No files above the size threshold found, or scan not started.
                </p>
              </div>
            ) : (
              <ul className="simple-list">
                {bigFiles.map((file, idx) => {
                  const isSelected = selectedPaths.has(file.path);
                  return (
                    <li
                      key={idx}
                      className={`file-item ${
                        isSelected ? "selected-item" : ""
                      }`}
                    >
                      <label className="checkbox-label">
                        <input
                          type="checkbox"
                          checked={isSelected}
                          onChange={() => toggleSelect(file.path)}
                        />
                        <span className="rank-badge">#{idx + 1}</span>
                        <span className="file-path">{file.path}</span>
                      </label>
                      <div className="image-meta">
                        <span className="size-badge big-size-badge">
                          {formatBytes(file.size)}
                        </span>
                        <button
                          className="icon-action-btn"
                          title="Copy path"
                          onClick={() =>
                            navigator.clipboard.writeText(file.path)
                          }
                        >
                          <Copy size={14} />
                        </button>
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        )}

        {mode === "temp_files" && (
          <div className="list-view">
            {tempFiles.length === 0 ? (
              <div className="empty-state">
                <Eraser size={48} opacity={0.3} />
                <p>No temporary files found, or scan not started.</p>
              </div>
            ) : (
              <ul className="simple-list">
                {tempFiles.map((file, idx) => {
                  const isSelected = selectedPaths.has(file.path);
                  return (
                    <li
                      key={idx}
                      className={`file-item ${
                        isSelected ? "selected-item" : ""
                      }`}
                    >
                      <label className="checkbox-label">
                        <input
                          type="checkbox"
                          checked={isSelected}
                          onChange={() => toggleSelect(file.path)}
                        />
                        <span className="temp-reason-badge">{file.reason}</span>
                        <span className="file-path">{file.path}</span>
                      </label>
                      <div className="image-meta">
                        <span className="size-badge">
                          {formatBytes(file.size)}
                        </span>
                        <button
                          className="icon-action-btn"
                          title="Copy path"
                          onClick={() =>
                            navigator.clipboard.writeText(file.path)
                          }
                        >
                          <Copy size={14} />
                        </button>
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        )}

        {mode === "broken_files" && (
          <div className="list-view">
            {brokenFiles.length === 0 ? (
              <div className="empty-state">
                <AlertTriangle size={48} opacity={0.3} />
                <p>No broken files found, or scan not started.</p>
              </div>
            ) : (
              <ul className="simple-list">
                {brokenFiles.map((file, idx) => {
                  const isSelected = selectedPaths.has(file.path);
                  return (
                    <li
                      key={idx}
                      className={`file-item ${
                        isSelected ? "selected-item" : ""
                      }`}
                    >
                      <label className="checkbox-label">
                        <input
                          type="checkbox"
                          checked={isSelected}
                          onChange={() => toggleSelect(file.path)}
                        />
                        <span className="broken-reason-badge">
                          {file.reason}
                        </span>
                        <span className="file-path">{file.path}</span>
                      </label>
                      <div className="image-meta">
                        <span className="size-badge">
                          {formatBytes(file.size)}
                        </span>
                        <button
                          className="icon-action-btn"
                          title="Copy path"
                          onClick={() =>
                            navigator.clipboard.writeText(file.path)
                          }
                        >
                          <Copy size={14} />
                        </button>
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        )}

        {mode === "empty_files" && (
          <div className="list-view">
            {emptyFiles.length === 0 ? (
              <div className="empty-state">
                <Trash2 size={48} opacity={0.3} />
                <p>No empty files found or scan not started.</p>
              </div>
            ) : (
              <ul className="simple-list">
                {emptyFiles.map((file, idx) => {
                  const isSelected = selectedPaths.has(file.path);
                  return (
                    <li
                      key={idx}
                      className={`file-item ${
                        isSelected ? "selected-item" : ""
                      }`}
                    >
                      <label className="checkbox-label">
                        <input
                          type="checkbox"
                          checked={isSelected}
                          onChange={() => toggleSelect(file.path)}
                        />
                        <span className="file-path">{file.path}</span>
                      </label>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        )}

        {mode === "empty_folders" && (
          <div className="list-view">
            {emptyFolders.length === 0 ? (
              <div className="empty-state">
                <FolderMinus size={48} opacity={0.3} />
                <p>No empty folders found or scan not started.</p>
              </div>
            ) : (
              <ul className="simple-list">
                {emptyFolders.map((folder, idx) => {
                  const isSelected = selectedPaths.has(folder);
                  return (
                    <li
                      key={idx}
                      className={`file-item ${
                        isSelected ? "selected-item" : ""
                      }`}
                    >
                      <label className="checkbox-label">
                        <input
                          type="checkbox"
                          checked={isSelected}
                          onChange={() => toggleSelect(folder)}
                        />
                        <span className="file-path">{folder}</span>
                      </label>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        )}
      </main>

      {/* Floating Bottom Action Bar when items are selected */}
      {selectedPaths.size > 0 && (
        <footer className="action-footer">
          <div className="footer-info">
            <span className="footer-count">
              <strong>{selectedPaths.size}</strong> items ready to clean
            </span>
            {selectedBytes > 0 && (
              <span className="footer-freed">
                Reclaimable: <strong>{formatBytes(selectedBytes)}</strong>
              </span>
            )}
          </div>
          <div className="footer-actions">
            <button
              className="danger-btn"
              onClick={handleMoveToTrash}
              disabled={cleaning}
            >
              {cleaning ? (
                <>
                  <RotateCw size={16} className="spin" />
                  <span>Moving to Trash...</span>
                </>
              ) : (
                <>
                  <Trash2 size={16} />
                  <span>Move to Recycle Bin</span>
                </>
              )}
            </button>
          </div>
        </footer>
      )}
      </div>
      </div>
    </div>
  );
}
