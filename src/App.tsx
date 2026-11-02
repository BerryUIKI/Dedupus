import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  FolderSearch,
  Files,
  FolderMinus,
  Trash2,
  Play,
  RotateCw,
  HardDrive,
  Copy,
  FolderOpen,
} from "lucide-react";
import "./App.css";

interface ScannedFile {
  path: string;
  size: number;
  modified_secs: number | null;
}

interface DuplicateGroup {
  file_size: number;
  hash: string;
  files: ScannedFile[];
}

type Mode = "duplicates" | "empty_files" | "empty_folders";

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`;
}

export default function App() {
  const [mode, setMode] = useState<Mode>("duplicates");
  const [scanPath, setScanPath] = useState("");
  const [minSize, setMinSize] = useState<number>(1024); // 1KB default
  const [algo, setAlgo] = useState("blake3");
  const [scanning, setScanning] = useState(false);
  const [statusMsg, setStatusMsg] = useState("");

  const [duplicates, setDuplicates] = useState<DuplicateGroup[]>([]);
  const [emptyFiles, setEmptyFiles] = useState<ScannedFile[]>([]);
  const [emptyFolders, setEmptyFolders] = useState<string[]>([]);

  const handleScan = async () => {
    if (!scanPath.trim()) {
      setStatusMsg("Please specify a directory path to scan.");
      return;
    }

    setScanning(true);
    setStatusMsg("Scanning in progress...");

    try {
      if (mode === "duplicates") {
        const res = await invoke<DuplicateGroup[]>("scan_duplicates", {
          directories: [scanPath.trim()],
          exclude: [],
          minSize: minSize > 0 ? minSize : null,
          maxSize: null,
          algo,
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

  return (
    <div className="app-container">
      {/* Top Header */}
      <header className="app-header">
        <div className="brand">
          <div className="brand-icon">
            <FolderSearch size={22} />
          </div>
          <div>
            <h1>Dedupus</h1>
            <p>Fast, Modern Duplicate Finder & Disk Cleaner</p>
          </div>
        </div>

        {/* Tab switchers */}
        <div className="tab-group">
          <button
            className={`tab-btn ${mode === "duplicates" ? "active" : ""}`}
            onClick={() => setMode("duplicates")}
          >
            <Files size={16} />
            <span>Duplicate Files</span>
          </button>
          <button
            className={`tab-btn ${mode === "empty_files" ? "active" : ""}`}
            onClick={() => setMode("empty_files")}
          >
            <Trash2 size={16} />
            <span>Empty Files</span>
          </button>
          <button
            className={`tab-btn ${mode === "empty_folders" ? "active" : ""}`}
            onClick={() => setMode("empty_folders")}
          >
            <FolderMinus size={16} />
            <span>Empty Folders</span>
          </button>
        </div>
      </header>

      {/* Control Bar */}
      <section className="control-bar">
        <div className="input-group flex-1">
          <FolderOpen size={18} className="input-icon" />
          <input
            type="text"
            placeholder="Enter directory path (e.g. C:\Downloads or F:\Projects)..."
            value={scanPath}
            onChange={(e) => setScanPath(e.target.value)}
            disabled={scanning}
          />
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
                disabled={scanning}
              />
            </div>
            <div className="input-group">
              <label>Algorithm:</label>
              <select
                value={algo}
                onChange={(e) => setAlgo(e.target.value)}
                disabled={scanning}
              >
                <option value="blake3">BLAKE3 (Default)</option>
                <option value="xxh3_128">XXH3-128</option>
                <option value="xxh3_64">XXH3-64</option>
              </select>
            </div>
          </>
        )}

        <button
          className="scan-btn"
          onClick={handleScan}
          disabled={scanning}
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

      {/* Status Bar */}
      {statusMsg && <div className="status-banner">{statusMsg}</div>}

      {/* Main Content Area */}
      <main className="results-container">
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
                      {group.files.map((file, fIdx) => (
                        <li key={fIdx} className="file-item">
                          <span className="file-path">{file.path}</span>
                          <button
                            className="icon-action-btn"
                            title="Copy path"
                            onClick={() =>
                              navigator.clipboard.writeText(file.path)
                            }
                          >
                            <Copy size={14} />
                          </button>
                        </li>
                      ))}
                    </ul>
                  </div>
                );
              })
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
                {emptyFiles.map((file, idx) => (
                  <li key={idx} className="simple-item">
                    <span>{file.path}</span>
                  </li>
                ))}
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
                {emptyFolders.map((folder, idx) => (
                  <li key={idx} className="simple-item">
                    <span>{folder}</span>
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}
      </main>
    </div>
  );
}
