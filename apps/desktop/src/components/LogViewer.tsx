import { useCallback, useEffect, useMemo, useState } from "react";
import { listLogFiles, readLogFile } from "../platform";
import type { LogFileInfo } from "../types";

type LogLevel = "all" | "info" | "warn" | "error";

export function LogViewer({ onClose }: { onClose: () => void }) {
  const [files, setFiles] = useState<LogFileInfo[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [lines, setLines] = useState<string[]>([]);
  const [totalLines, setTotalLines] = useState(0);
  const [offset, setOffset] = useState(0);
  const [levelFilter, setLevelFilter] = useState<LogLevel>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const pageSize = 200;

  useEffect(() => {
    void listLogFiles().then(setFiles);
  }, []);

  const loadLines = useCallback(async (path: string, startOffset: number) => {
    const result = await readLogFile(path, startOffset, pageSize);
    setLines(result.lines);
    setTotalLines(result.total);
    setOffset(startOffset);
  }, []);

  useEffect(() => {
    if (selectedPath) {
      setOffset(0);
      void loadLines(selectedPath, 0);
    }
  }, [selectedPath, loadLines]);

  const filteredLines = useMemo(() => {
    return lines.filter((line) => {
      try {
        const parsed = JSON.parse(line);
        if (levelFilter !== "all" && parsed.level !== levelFilter) return false;
        if (
          searchQuery &&
          !JSON.stringify(parsed)
            .toLowerCase()
            .includes(searchQuery.toLowerCase())
        )
          return false;
        return true;
      } catch {
        if (
          searchQuery &&
          !line.toLowerCase().includes(searchQuery.toLowerCase())
        )
          return false;
        return true;
      }
    });
  }, [lines, levelFilter, searchQuery]);

  const selectedFile = files.find((f) => f.path === selectedPath);

  return (
    <div className="modal-backdrop" role="presentation" onClick={onClose}>
      <section
        className="log-viewer"
        role="dialog"
        aria-modal="true"
        onClick={(event) => event.stopPropagation()}
      >
        <header>
          <h2>Log Viewer</h2>
          <button className="secondary-button" onClick={onClose}>
            Close
          </button>
        </header>
        <div className="log-viewer-body">
          <aside className="log-sidebar">
            <h3>Log Files</h3>
            <div className="log-file-list">
              {files.map((file) => (
                <button
                  key={file.path}
                  className={`log-file-row ${file.path === selectedPath ? "active" : ""}`}
                  onClick={() => setSelectedPath(file.path)}
                >
                  <span className="log-file-name">{file.name}</span>
                  <span className="log-file-size">
                    {(file.size / 1024).toFixed(1)} KB
                  </span>
                  <span className="log-file-date">
                    {new Date(file.modifiedAt).toLocaleDateString()}
                  </span>
                </button>
              ))}
              {files.length === 0 && (
                <p className="log-empty">No log files found.</p>
              )}
            </div>
          </aside>
          <main className="log-content">
            {selectedFile ? (
              <>
                <div className="log-toolbar">
                  <div className="log-controls">
                    <select
                      value={levelFilter}
                      onChange={(e) =>
                        setLevelFilter(e.target.value as LogLevel)
                      }
                    >
                      <option value="all">All Levels</option>
                      <option value="info">Info</option>
                      <option value="warn">Warn</option>
                      <option value="error">Error</option>
                    </select>
                    <input
                      type="text"
                      placeholder="Search log contents..."
                      value={searchQuery}
                      onChange={(e) => setSearchQuery(e.target.value)}
                    />
                  </div>
                  <div className="log-pagination">
                    <span>
                      Lines {offset + 1}-
                      {Math.min(offset + pageSize, totalLines)} of {totalLines}
                    </span>
                    <button
                      disabled={offset === 0}
                      onClick={() =>
                        void loadLines(
                          selectedPath!,
                          Math.max(0, offset - pageSize),
                        )
                      }
                    >
                      Prev
                    </button>
                    <button
                      disabled={offset + pageSize >= totalLines}
                      onClick={() =>
                        void loadLines(selectedPath!, offset + pageSize)
                      }
                    >
                      Next
                    </button>
                  </div>
                </div>
                <pre className="log-lines">
                  {filteredLines.length === 0 ? (
                    <span className="log-empty">No matching log entries.</span>
                  ) : (
                    filteredLines.map((line, index) => (
                      <LogLine key={offset + index} line={line} />
                    ))
                  )}
                </pre>
              </>
            ) : (
              <div className="log-empty-state">
                <p>Select a log file from the sidebar.</p>
              </div>
            )}
          </main>
        </div>
      </section>
    </div>
  );
}

function LogLine({ line }: { line: string }) {
  let level = "info";
  let content = line;
  try {
    const parsed = JSON.parse(line);
    level = parsed.level || "info";
    content = JSON.stringify(parsed, null, 2);
  } catch {
    // Plain text line
  }
  return (
    <div className={`log-line log-level-${level}`}>
      <code>{content}</code>
    </div>
  );
}
