import { useState, useEffect, useMemo, useRef } from "react";
import type { ReactNode, RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useVirtualizer } from "@tanstack/react-virtual";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { confirm as confirmDialog, open } from "@tauri-apps/plugin-dialog";
import type { LucideIcon } from "lucide-react";
import {
  FolderSearch,
  Files,
  FileWarning,
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
  Sun,
  Moon,
  ExternalLink,
  Link2,
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

interface BadExtensionEntry {
  path: string;
  size: number;
  modified_secs: number | null;
  extension: string;
  detected_extension: string;
  detected_mime: string;
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

interface ScanOutcome<T> {
  results: T;
  cancelled: boolean;
}

type ScanEventName =
  | "stage"
  | "directory"
  | "discovered"
  | "hashing_started"
  | "hashing";

interface ScanEvent {
  scanId: number;
  kind: string;
  event: ScanEventName;
  stage?: string;
  path?: string;
  count?: number;
  processed?: number;
  total?: number;
}

interface ScanProgress {
  stage?: string;
  path?: string;
  discovered?: number;
  processed?: number;
  total?: number;
}

interface ContextMenuState {
  x: number;
  y: number;
  path: string;
  groupPaths?: string[];
  isDirectory?: boolean;
}

type Mode =
  | "duplicates"
  | "similar_images"
  | "similar_audio"
  | "similar_video"
  | "big_files"
  | "temp_files"
  | "broken_files"
  | "bad_extensions"
  | "empty_files"
  | "empty_folders";

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`;
}

/**
 * Renders only the visible slice of a large result list.
 * Rows are absolutely positioned inside a spacer with the virtualizer's
 * total height, and measured individually because cards have variable height.
 */
function VirtualList<T>({
  scrollRef,
  items,
  estimateSize,
  renderItem,
  rowClassName,
  emptyState,
}: {
  scrollRef: RefObject<HTMLElement | null>;
  items: T[];
  estimateSize: (index: number) => number;
  renderItem: (item: T, index: number) => ReactNode;
  rowClassName: string;
  emptyState: ReactNode;
}) {
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize,
    overscan: 8,
  });

  if (items.length === 0) {
    return <>{emptyState}</>;
  }

  return (
    <div
      className="virtual-list"
      style={{ height: virtualizer.getTotalSize() }}
    >
      {virtualizer.getVirtualItems().map((virtualRow) => (
        <div
          key={virtualRow.key}
          ref={virtualizer.measureElement}
          data-index={virtualRow.index}
          className={`virtual-row ${rowClassName}`}
          style={{ transform: `translateY(${virtualRow.start}px)` }}
        >
          {renderItem(items[virtualRow.index], virtualRow.index)}
        </div>
      ))}
    </div>
  );
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
      "Images, archives, PDFs and audio validated for corruption; broken symlinks and Windows shortcuts included.",
  },
  bad_extensions: {
    title: "Bad Extensions",
    subtitle:
      "Files whose leading bytes reveal a different format than their extension claims.",
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
      { mode: "bad_extensions", label: "Bad Extensions", icon: FileWarning },
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
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const activeScanId = useRef(0);
  const resultsRef = useRef<HTMLElement | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);

  const [duplicates, setDuplicates] = useState<DuplicateGroup[]>([]);
  const [similarImages, setSimilarImages] = useState<SimilarImageGroup[]>([]);
  const [similarAudio, setSimilarAudio] = useState<SimilarAudioGroup[]>([]);
  const [similarVideos, setSimilarVideos] = useState<SimilarVideoGroup[]>([]);
  const [bigFiles, setBigFiles] = useState<ScannedFile[]>([]);
  const [tempFiles, setTempFiles] = useState<TempFileEntry[]>([]);
  const [brokenFiles, setBrokenFiles] = useState<BrokenFileEntry[]>([]);
  const [badExtensions, setBadExtensions] = useState<BadExtensionEntry[]>([]);
  const [emptyFiles, setEmptyFiles] = useState<ScannedFile[]>([]);
  const [emptyFolders, setEmptyFolders] = useState<string[]>([]);

  // Set of selected file paths for actions
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [isMaximized, setIsMaximized] = useState(false);
  const [theme, setTheme] = useState<"light" | "dark">(() => {
    try {
      const saved = localStorage.getItem("dedupus-theme");
      if (saved === "light" || saved === "dark") return saved;
    } catch {
      // ignore
    }
    return "light";
  });

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem("dedupus-theme", theme);
    } catch {
      // ignore
    }
  }, [theme]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    (async () => {
      try {
        unlisten = await listen<ScanEvent>("scan-progress", (event) => {
          const payload = event.payload;
          if (payload.scanId !== activeScanId.current) return;

          setProgress((prev) => {
            const next: ScanProgress = { ...(prev ?? {}) };
            switch (payload.event) {
              case "stage":
                if (payload.stage) next.stage = payload.stage;
                break;
              case "directory":
                if (payload.path) next.path = payload.path;
                break;
              case "discovered":
                if (payload.count != null) next.discovered = payload.count;
                break;
              case "hashing_started":
                next.processed = 0;
                next.total = payload.total;
                break;
              case "hashing":
                next.processed = payload.processed;
                next.total = payload.total;
                break;
            }
            return next;
          });
        });
      } catch {
        // Browser preview without the Tauri runtime
      }
    })();

    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  useEffect(() => {
    resultsRef.current?.scrollTo({ top: 0 });
  }, [mode]);

  useEffect(() => {
    if (!contextMenu) return;

    const close = () => setContextMenu(null);
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };

    window.addEventListener("click", close);
    window.addEventListener("contextmenu", close);
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);

    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("contextmenu", close);
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  }, [contextMenu]);

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
    if (scanning) return;

    const scanId = Date.now();
    activeScanId.current = scanId;
    setScanning(true);
    setCancelling(false);
    setProgress({ stage: "Preparing scan…" });
    setStatusMsg("Scanning in progress...");
    setSelectedPaths(new Set());

    // Clear the current view so stale results are not mistaken for this scan.
    if (mode === "duplicates") setDuplicates([]);
    else if (mode === "similar_images") setSimilarImages([]);
    else if (mode === "similar_audio") setSimilarAudio([]);
    else if (mode === "similar_video") setSimilarVideos([]);
    else if (mode === "big_files") setBigFiles([]);
    else if (mode === "temp_files") setTempFiles([]);
    else if (mode === "broken_files") setBrokenFiles([]);
    else if (mode === "bad_extensions") setBadExtensions([]);
    else if (mode === "empty_files") setEmptyFiles([]);
    else if (mode === "empty_folders") setEmptyFolders([]);

    try {
      if (mode === "duplicates") {
        const outcome = await invoke<ScanOutcome<DuplicateGroup[]>>(
          "scan_duplicates",
          {
            scanId,
            directories: [scanPath.trim()],
            exclude: [],
            minSize: minSize > 0 ? minSize : null,
            maxSize: null,
            algo,
            useCache,
          }
        );
        if (outcome.cancelled) {
          setStatusMsg("Scan cancelled.");
          return;
        }
        const res = outcome.results;
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
        const outcome = await invoke<ScanOutcome<SimilarImageGroup[]>>(
          "scan_similar_images",
          {
            scanId,
            directories: [scanPath.trim()],
            exclude: [],
            distance: hammingDistance,
            algo: imageAlgo,
            useCache,
          }
        );
        if (outcome.cancelled) {
          setStatusMsg("Scan cancelled.");
          return;
        }
        const res = outcome.results;
        setSimilarImages(res);
        const totalImages = res.reduce((acc, g) => acc + g.images.length, 0);
        setStatusMsg(
          `Scan completed: Found ${res.length} similar image groups (${totalImages} total images)`
        );
      } else if (mode === "similar_audio") {
        const outcome = await invoke<ScanOutcome<SimilarAudioGroup[]>>(
          "scan_similar_audio",
          {
            scanId,
            directories: [scanPath.trim()],
            exclude: [],
            maxDurationDiff: durationDiff,
          }
        );
        if (outcome.cancelled) {
          setStatusMsg("Scan cancelled.");
          return;
        }
        const res = outcome.results;
        setSimilarAudio(res);
        const totalTracks = res.reduce((acc, g) => acc + g.tracks.length, 0);
        setStatusMsg(
          `Scan completed: Found ${res.length} similar audio groups (${totalTracks} total tracks)`
        );
      } else if (mode === "similar_video") {
        const outcome = await invoke<ScanOutcome<SimilarVideoGroup[]>>(
          "scan_similar_videos",
          {
            scanId,
            directories: [scanPath.trim()],
            exclude: [],
            maxDurationDiff: durationDiff,
          }
        );
        if (outcome.cancelled) {
          setStatusMsg("Scan cancelled.");
          return;
        }
        const res = outcome.results;
        setSimilarVideos(res);
        const totalVideos = res.reduce((acc, g) => acc + g.videos.length, 0);
        setStatusMsg(
          `Scan completed: Found ${res.length} similar video groups (${totalVideos} total videos)`
        );
      } else if (mode === "big_files") {
        const outcome = await invoke<ScanOutcome<ScannedFile[]>>(
          "scan_big_files",
          {
            scanId,
            directories: [scanPath.trim()],
            exclude: [],
            minSize: Math.max(1, Math.round(bigMinMb * 1024 * 1024)),
            maxResults: Math.max(1, bigLimit),
          }
        );
        if (outcome.cancelled) {
          setStatusMsg("Scan cancelled.");
          return;
        }
        const res = outcome.results;
        setBigFiles(res);
        const totalBytes = res.reduce((acc, f) => acc + f.size, 0);
        setStatusMsg(
          `Scan completed: Found ${res.length} large files (${formatBytes(
            totalBytes
          )} total)`
        );
      } else if (mode === "temp_files") {
        const outcome = await invoke<ScanOutcome<TempFileEntry[]>>(
          "scan_temp_files",
          {
            scanId,
            directories: [scanPath.trim()],
            exclude: [],
            minSize: Math.max(0, Math.round(tempMinKb * 1024)),
          }
        );
        if (outcome.cancelled) {
          setStatusMsg("Scan cancelled.");
          return;
        }
        const res = outcome.results;
        setTempFiles(res);
        const totalBytes = res.reduce((acc, f) => acc + f.size, 0);
        setStatusMsg(
          `Scan completed: Found ${res.length} temporary files (${formatBytes(
            totalBytes
          )} total)`
        );
      } else if (mode === "broken_files") {
        const outcome = await invoke<ScanOutcome<BrokenFileEntry[]>>(
          "scan_broken_files",
          {
            scanId,
            directories: [scanPath.trim()],
            exclude: [],
          }
        );
        if (outcome.cancelled) {
          setStatusMsg("Scan cancelled.");
          return;
        }
        const res = outcome.results;
        setBrokenFiles(res);
        setStatusMsg(`Scan completed: Found ${res.length} broken files`);
      } else if (mode === "bad_extensions") {
        const outcome = await invoke<ScanOutcome<BadExtensionEntry[]>>(
          "scan_bad_extensions",
          {
            scanId,
            directories: [scanPath.trim()],
            exclude: [],
            minSize: 1,
          }
        );
        if (outcome.cancelled) {
          setStatusMsg("Scan cancelled.");
          return;
        }
        const res = outcome.results;
        setBadExtensions(res);
        setStatusMsg(
          `Scan completed: Found ${res.length} files with mismatched extensions`
        );
      } else if (mode === "empty_files") {
        const outcome = await invoke<ScanOutcome<ScannedFile[]>>(
          "scan_empty_files",
          {
            scanId,
            directories: [scanPath.trim()],
            exclude: [],
          }
        );
        if (outcome.cancelled) {
          setStatusMsg("Scan cancelled.");
          return;
        }
        const res = outcome.results;
        setEmptyFiles(res);
        setStatusMsg(`Scan completed: Found ${res.length} empty files`);
      } else if (mode === "empty_folders") {
        const outcome = await invoke<ScanOutcome<string[]>>(
          "scan_empty_folders",
          {
            scanId,
            directories: [scanPath.trim()],
            exclude: [],
          }
        );
        if (outcome.cancelled) {
          setStatusMsg("Scan cancelled.");
          return;
        }
        const res = outcome.results;
        setEmptyFolders(res);
        setStatusMsg(`Scan completed: Found ${res.length} empty folders`);
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Scan failed: ${msg}`);
    } finally {
      activeScanId.current = 0;
      setScanning(false);
      setCancelling(false);
      setProgress(null);
    }
  };

  const handleCancelScan = async () => {
    if (cancelling) return;
    setCancelling(true);
    setProgress((prev) => ({ ...(prev ?? {}), stage: "Cancelling…" }));
    setStatusMsg("Cancelling scan…");
    try {
      await invoke("cancel_scan");
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Cancel failed: ${msg}`);
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
    } else if (mode === "bad_extensions") {
      for (const file of badExtensions) {
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
    } else if (mode === "bad_extensions") {
      for (const file of badExtensions) {
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
    badExtensions,
    mode,
  ]);

  const pathsForAction = (path: string): string[] =>
    selectedPaths.has(path) && selectedPaths.size > 1
      ? Array.from(selectedPaths)
      : [path];

  const openContextMenu = (
    event: React.MouseEvent,
    path: string,
    options?: { groupPaths?: string[]; isDirectory?: boolean }
  ) => {
    event.preventDefault();
    event.stopPropagation();
    const menuWidth = 240;
    const menuHeight = 320;
    setContextMenu({
      x: Math.max(8, Math.min(event.clientX, window.innerWidth - menuWidth - 8)),
      y: Math.max(8, Math.min(event.clientY, window.innerHeight - menuHeight - 8)),
      path,
      groupPaths: options?.groupPaths,
      isDirectory: options?.isDirectory,
    });
  };

  const applyCleanReport = (
    report: CleanReport,
    action: "trash" | "delete" | "hardlink"
  ) => {
    const succSet = new Set(report.successful);

    // Remove cleaned files from state
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
    } else if (mode === "bad_extensions") {
      setBadExtensions((prev) => prev.filter((f) => !succSet.has(f.path)));
    }

    setSelectedPaths(new Set());

    const count = report.successful.length;
    const verb =
      action === "trash"
        ? "Moved to Recycle Bin"
        : action === "delete"
          ? "Permanently deleted"
          : "Replaced with hard links";
    setStatusMsg(
      `${verb} ${count} item${count === 1 ? "" : "s"} (${formatBytes(
        report.bytes_freed
      )} freed).`
    );

    if (report.failed.length > 0) {
      const reason = report.failed[0]?.[1] ?? "unknown error";
      setStatusMsg((prev) => `${prev} ${report.failed.length} failed: ${reason}`);
    }
  };

  // Trash action
  const handleMoveToTrash = async () => {
    if (selectedPaths.size === 0) return;
    setCleaning(true);

    try {
      const report = await invoke<CleanReport>("delete_to_trash", {
        paths: Array.from(selectedPaths),
      });
      applyCleanReport(report, "trash");
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Trash action failed: ${msg}`);
    } finally {
      setCleaning(false);
    }
  };

  const handleTrashPaths = async (paths: string[]) => {
    setContextMenu(null);
    setCleaning(true);

    try {
      const report = await invoke<CleanReport>("delete_to_trash", { paths });
      applyCleanReport(report, "trash");
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Trash action failed: ${msg}`);
    } finally {
      setCleaning(false);
    }
  };

  const handlePermanentDelete = async (paths: string[]) => {
    setContextMenu(null);

    let confirmed = false;
    try {
      confirmed = await confirmDialog(
        `Permanently delete ${
          paths.length === 1 ? paths[0] : `${paths.length} items`
        }?\n\nThis bypasses the Recycle Bin and cannot be undone.`,
        { title: "Delete permanently", kind: "warning" }
      );
    } catch {
      return;
    }
    if (!confirmed) return;

    setCleaning(true);
    try {
      const report = await invoke<CleanReport>("delete_permanently", { paths });
      applyCleanReport(report, "delete");
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Permanent delete failed: ${msg}`);
    } finally {
      setCleaning(false);
    }
  };

  const handleHardlink = async (path: string, groupPaths: string[]) => {
    setContextMenu(null);
    const target = groupPaths.find((candidate) => candidate !== path);
    if (!target) return;

    let confirmed = false;
    try {
      confirmed = await confirmDialog(
        `Replace this duplicate with a hard link to:\n${target}\n\nThe path stays the same and no extra disk space is used. Both files must be on the same volume.`,
        { title: "Replace with hard link", kind: "warning" }
      );
    } catch {
      return;
    }
    if (!confirmed) return;

    setCleaning(true);
    try {
      const report = await invoke<CleanReport>("replace_with_hardlinks", {
        target,
        duplicates: [path],
      });
      applyCleanReport(report, "hardlink");
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Hard link failed: ${msg}`);
    } finally {
      setCleaning(false);
    }
  };

  const handleOpenPath = async (path: string) => {
    setContextMenu(null);
    try {
      await openPath(path);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Open failed: ${msg}`);
    }
  };

  const handleRevealPath = async (path: string) => {
    setContextMenu(null);
    try {
      await revealItemInDir(path);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Reveal failed: ${msg}`);
    }
  };

  const handleCopyPath = async (path: string) => {
    setContextMenu(null);
    try {
      await navigator.clipboard.writeText(path);
      setStatusMsg("Path copied to clipboard.");
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMsg(`Copy failed: ${msg}`);
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

  const ctxPaths = contextMenu ? pathsForAction(contextMenu.path) : [];

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
                    disabled={scanning || cleaning}
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

          <div className="topbar-actions">
            <button
              className="theme-toggle"
              onClick={() =>
                setTheme((prev) => (prev === "light" ? "dark" : "light"))
              }
              title={
                theme === "light" ? "Switch to dark theme" : "Switch to light theme"
              }
            >
              {theme === "light" ? <Moon size={14} /> : <Sun size={14} />}
              <span>{theme === "light" ? "Dark" : "Light"}</span>
            </button>

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

      {/* Live scan progress with cooperative cancellation */}
      {scanning && (
        <div className="scan-progress">
          <div className="scan-progress-head">
            <div className="scan-progress-label">
              <RotateCw size={13} className="spin" />
              <span className="scan-progress-stage">
                {progress?.stage ?? "Scanning…"}
              </span>
              {progress?.discovered != null && (
                <span className="scan-progress-count">
                  {progress.discovered.toLocaleString()} items found
                </span>
              )}
            </div>
            <button
              className="cancel-scan-btn"
              onClick={handleCancelScan}
              disabled={cancelling}
            >
              <X size={13} />
              <span>{cancelling ? "Cancelling…" : "Cancel"}</span>
            </button>
          </div>

          <div className="progress-track">
            {progress?.total ? (
              <div
                className="progress-bar determinate"
                style={{
                  width: `${Math.min(
                    100,
                    Math.round(((progress.processed ?? 0) / progress.total) * 100)
                  )}%`,
                }}
              />
            ) : (
              <div className="progress-bar" />
            )}
          </div>

          <div className="scan-progress-meta">
            {progress?.total != null && (
              <span>
                {progress.processed ?? 0} / {progress.total} processed
              </span>
            )}
            {progress?.path && (
              <span className="scan-progress-path" title={progress.path}>
                {progress.path}
              </span>
            )}
          </div>
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
        (mode === "bad_extensions" && badExtensions.length > 0) ||
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
                mode === "broken_files" ||
                mode === "bad_extensions" ||
                mode === "empty_files" ||
                mode === "empty_folders"
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
        ref={resultsRef}
        className={`results-container ${
          selectedPaths.size > 0 ? "with-footer" : ""
        }`}
      >
        {mode === "duplicates" && (
          <VirtualList
            scrollRef={resultsRef}
            items={duplicates}
            rowClassName="virtual-group-row"
            estimateSize={(index) => 95 + duplicates[index].files.length * 45}
            emptyState={
              <div className="list-view">
                <div className="empty-state">
                  <HardDrive size={48} opacity={0.3} />
                  <p>No duplicates found or no scan executed yet.</p>
                </div>
              </div>
            }
            renderItem={(group, gIdx) => {
              const wasted = group.file_size * (group.files.length - 1);
              const groupPaths = group.files.map((file) => file.path);
              return (
                <div className="result-card">
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
                          onContextMenu={(event) =>
                            openContextMenu(event, file.path, { groupPaths })
                          }
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
            }}
          />
        )}

        {mode === "similar_images" && (
          <VirtualList
            scrollRef={resultsRef}
            items={similarImages}
            rowClassName="virtual-group-row"
            estimateSize={(index) => 95 + similarImages[index].images.length * 45}
            emptyState={
              <div className="list-view">
                <div className="empty-state">
                  <Images size={48} opacity={0.3} />
                  <p>No similar images found or scan not started.</p>
                </div>
              </div>
            }
            renderItem={(group, gIdx) => (
              <div className="result-card">
                <div className="card-header">
                  <span className="badge purple-badge">
                    Image Group #{gIdx + 1}
                  </span>
                  <span className="meta">{group.images.length} candidates</span>
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
                        onContextMenu={(event) =>
                          openContextMenu(event, img.path)
                        }
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
            )}
          />
        )}

        {mode === "similar_audio" && (
          <VirtualList
            scrollRef={resultsRef}
            items={similarAudio}
            rowClassName="virtual-group-row"
            estimateSize={(index) => 95 + similarAudio[index].tracks.length * 45}
            emptyState={
              <div className="list-view">
                <div className="empty-state">
                  <Music size={48} opacity={0.3} />
                  <p>No similar audio tracks found or scan not started.</p>
                </div>
              </div>
            }
            renderItem={(group, gIdx) => (
              <div className="result-card">
                <div className="card-header">
                  <span className="badge teal-badge">
                    Audio Group #{gIdx + 1}
                  </span>
                  <span className="meta">{group.tracks.length} tracks</span>
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
                        onContextMenu={(event) =>
                          openContextMenu(event, track.path)
                        }
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
            )}
          />
        )}

        {mode === "similar_video" && (
          <VirtualList
            scrollRef={resultsRef}
            items={similarVideos}
            rowClassName="virtual-group-row"
            estimateSize={(index) => 95 + similarVideos[index].videos.length * 45}
            emptyState={
              <div className="list-view">
                <div className="empty-state">
                  <Video size={48} opacity={0.3} />
                  <p>No similar videos found or scan not started.</p>
                </div>
              </div>
            }
            renderItem={(group, gIdx) => (
              <div className="result-card">
                <div className="card-header">
                  <span className="badge amber-badge">
                    Video Group #{gIdx + 1}
                  </span>
                  <span className="meta">{group.videos.length} videos</span>
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
                        onContextMenu={(event) =>
                          openContextMenu(event, vid.path)
                        }
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
            )}
          />
        )}

        {mode === "big_files" && (
          <VirtualList
            scrollRef={resultsRef}
            items={bigFiles}
            rowClassName="virtual-item-row"
            estimateSize={() => 52}
            emptyState={
              <div className="list-view">
                <div className="empty-state">
                  <ArrowDownWideNarrow size={48} opacity={0.3} />
                  <p>
                    No files above the size threshold found, or scan not
                    started.
                  </p>
                </div>
              </div>
            }
            renderItem={(file, idx) => {
              const isSelected = selectedPaths.has(file.path);
              return (
                <div
                  className={`file-item ${
                    isSelected ? "selected-item" : ""
                  }`}
                  onContextMenu={(event) =>
                    openContextMenu(event, file.path)
                  }
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
                </div>
              );
            }}
          />
        )}

        {mode === "temp_files" && (
          <VirtualList
            scrollRef={resultsRef}
            items={tempFiles}
            rowClassName="virtual-item-row"
            estimateSize={() => 52}
            emptyState={
              <div className="list-view">
                <div className="empty-state">
                  <Eraser size={48} opacity={0.3} />
                  <p>No temporary files found, or scan not started.</p>
                </div>
              </div>
            }
            renderItem={(file) => {
              const isSelected = selectedPaths.has(file.path);
              return (
                <div
                  className={`file-item ${
                    isSelected ? "selected-item" : ""
                  }`}
                  onContextMenu={(event) =>
                    openContextMenu(event, file.path)
                  }
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
                </div>
              );
            }}
          />
        )}

        {mode === "broken_files" && (
          <VirtualList
            scrollRef={resultsRef}
            items={brokenFiles}
            rowClassName="virtual-item-row"
            estimateSize={() => 52}
            emptyState={
              <div className="list-view">
                <div className="empty-state">
                  <AlertTriangle size={48} opacity={0.3} />
                  <p>No broken files found, or scan not started.</p>
                </div>
              </div>
            }
            renderItem={(file) => {
              const isSelected = selectedPaths.has(file.path);
              return (
                <div
                  className={`file-item ${
                    isSelected ? "selected-item" : ""
                  }`}
                  onContextMenu={(event) =>
                    openContextMenu(event, file.path)
                  }
                >
                  <label className="checkbox-label">
                    <input
                      type="checkbox"
                      checked={isSelected}
                      onChange={() => toggleSelect(file.path)}
                    />
                    <span className="broken-reason-badge">{file.reason}</span>
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
                </div>
              );
            }}
          />
        )}

        {mode === "bad_extensions" && (
          <VirtualList
            scrollRef={resultsRef}
            items={badExtensions}
            rowClassName="virtual-item-row"
            estimateSize={() => 52}
            emptyState={
              <div className="list-view">
                <div className="empty-state">
                  <FileWarning size={48} opacity={0.3} />
                  <p>No mismatched extensions found, or scan not started.</p>
                </div>
              </div>
            }
            renderItem={(file) => {
              const isSelected = selectedPaths.has(file.path);
              return (
                <div
                  className={`file-item ${
                    isSelected ? "selected-item" : ""
                  }`}
                  onContextMenu={(event) =>
                    openContextMenu(event, file.path)
                  }
                >
                  <label className="checkbox-label">
                    <input
                      type="checkbox"
                      checked={isSelected}
                      onChange={() => toggleSelect(file.path)}
                    />
                    <span className="extension-badge">.{file.extension}</span>
                    <span
                      className="detected-badge"
                      title={`Detected: ${file.detected_mime}`}
                    >
                      content: {file.detected_extension}
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
                </div>
              );
            }}
          />
        )}

        {mode === "empty_files" && (
          <VirtualList
            scrollRef={resultsRef}
            items={emptyFiles}
            rowClassName="virtual-item-row"
            estimateSize={() => 52}
            emptyState={
              <div className="list-view">
                <div className="empty-state">
                  <Trash2 size={48} opacity={0.3} />
                  <p>No empty files found or scan not started.</p>
                </div>
              </div>
            }
            renderItem={(file) => {
              const isSelected = selectedPaths.has(file.path);
              return (
                <div
                  className={`file-item ${
                    isSelected ? "selected-item" : ""
                  }`}
                  onContextMenu={(event) =>
                    openContextMenu(event, file.path)
                  }
                >
                  <label className="checkbox-label">
                    <input
                      type="checkbox"
                      checked={isSelected}
                      onChange={() => toggleSelect(file.path)}
                    />
                    <span className="file-path">{file.path}</span>
                  </label>
                </div>
              );
            }}
          />
        )}

        {mode === "empty_folders" && (
          <VirtualList
            scrollRef={resultsRef}
            items={emptyFolders}
            rowClassName="virtual-item-row"
            estimateSize={() => 52}
            emptyState={
              <div className="list-view">
                <div className="empty-state">
                  <FolderMinus size={48} opacity={0.3} />
                  <p>No empty folders found or scan not started.</p>
                </div>
              </div>
            }
            renderItem={(folder) => {
              const isSelected = selectedPaths.has(folder);
              return (
                <div
                  className={`file-item ${
                    isSelected ? "selected-item" : ""
                  }`}
                  onContextMenu={(event) =>
                    openContextMenu(event, folder, { isDirectory: true })
                  }
                >
                  <label className="checkbox-label">
                    <input
                      type="checkbox"
                      checked={isSelected}
                      onChange={() => toggleSelect(folder)}
                    />
                    <span className="file-path">{folder}</span>
                  </label>
                </div>
              );
            }}
          />
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

      {contextMenu && (
        <div
          className="context-menu"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onContextMenu={(event) => event.preventDefault()}
        >
          <button
            className="context-item"
            onClick={() => handleOpenPath(contextMenu.path)}
          >
            <ExternalLink size={14} />
            <span>
              {contextMenu.isDirectory ? "Open folder" : "Open file"}
            </span>
          </button>
          <button
            className="context-item"
            onClick={() => handleRevealPath(contextMenu.path)}
          >
            <FolderOpen size={14} />
            <span>Reveal in Explorer</span>
          </button>
          <button
            className="context-item"
            onClick={() => handleCopyPath(contextMenu.path)}
          >
            <Copy size={14} />
            <span>Copy path</span>
          </button>

          <div className="context-separator" />

          {mode === "duplicates" &&
            contextMenu.groupPaths &&
            contextMenu.groupPaths.length >= 2 && (
              <button
                className="context-item"
                onClick={() =>
                  handleHardlink(contextMenu.path, contextMenu.groupPaths!)
                }
              >
                <Link2 size={14} />
                <span>Replace with hard link</span>
              </button>
            )}

          <button
            className="context-item"
            onClick={() => handleTrashPaths(ctxPaths)}
            disabled={cleaning}
          >
            <Trash2 size={14} />
            <span>
              Move to Recycle Bin
              {ctxPaths.length > 1 ? ` (${ctxPaths.length})` : ""}
            </span>
          </button>
          <button
            className="context-item danger"
            onClick={() => handlePermanentDelete(ctxPaths)}
            disabled={cleaning}
          >
            <AlertTriangle size={14} />
            <span>
              Delete permanently
              {ctxPaths.length > 1 ? ` (${ctxPaths.length})` : ""}
            </span>
          </button>
        </div>
      )}
      </div>
      </div>
    </div>
  );
}
