import {
  currentMonitor,
  Effect,
  getCurrentWindow,
  PhysicalPosition,
} from "@tauri-apps/api/window";
// imports
import { anchorOf } from "./components/Popover";
import type { Anchor } from "./components/Popover";
import "./styles/index.css"; // replaces import "./App.css";
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowLeft,
  Bell,
  CircleHelp,
  ClipboardPaste,
  Copy,
  File as FileIcon,
  FileText,
  FolderOpen,
  History,
  Image as ImageIcon,
  MonitorSmartphone,
  PencilLine,
  RefreshCw,
  Send,
  Settings,
  Trash2,
  Type,
  Upload,
  X,
} from "lucide-react";
import { showToast, subscribeToast, ToastState } from "./components/toast";
import { Toast } from "./components/Toast.tsx";
import { ScrollFade, ScrollFadeTextarea } from "./components/ScrollFade";
import {
  AvailableRow,
  DeviceDetail,
  DeviceIdentity,
  EmptyState,
  PairedRow,
} from "./components/DeviceParts";
import { ClipRow, CollapsibleList } from "./components/ClipParts";
import { OWN_IDENTITY, PAIRING_STATE } from "./types";
import type {
  AvailableDevice,
  ClipItem,
  DeviceType,
  DiagnosticSnapshot,
  OwnIdentity,
  PairedDevice,
  PairingEventPayload,
  PickedFile,
  Screen,
  TransferProgress,
  SendContent,
} from "./types";
import { formatDataSize, visibleDiagnosticLogs } from "./utils";

function App() {
  const appWindow = getCurrentWindow();
  const [screen, setScreen] = useState<Screen>("devices");
  const [paired, setPaired] = useState<PairedDevice[]>([]);
  const [available, setAvailable] = useState<AvailableDevice[]>([]);
  const [history, setHistory] = useState<ClipItem[]>([]);
  const [showIdentity, setShowIdentity] = useState(false);
  const [showIdentityDisabled, setShowIdentityDisabled] = useState(true);
  const [detailFingerprint, setDetailFingerprint] = useState<string | null>(
    null,
  );
  const [sendContent, setSendContent] = useState<SendContent | null>(null);

  const [editingText, setEditingText] = useState(false);
  const [textDraft, setTextDraft] = useState("");
  const [outgoingFiles, setOutgoingFiles] = useState<PickedFile[]>([]);
  const outgoingRef = useRef<PickedFile[]>([]);
  useEffect(() => {
    outgoingRef.current = outgoingFiles;
  }, [outgoingFiles]);
  const [isFileDragActive, setIsFileDragActive] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState<
    Record<string, TransferProgress>
  >({});
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [sendingReadyItems, setSendingReadyItems] = useState(false);
  const [diagnostics, setDiagnostics] = useState<DiagnosticSnapshot | null>(
    null,
  );
  const [diagnosticsClearMarker, setDiagnosticsClearMarker] = useState<
    string | null
  >(null);
  const [diagnosticsLoading, setDiagnosticsLoading] = useState(false);
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null);

  const [toast, setToast] = useState<ToastState | null>(null);
  const [ownIdentity, setOwnIdentity] = useState<OwnIdentity>(OWN_IDENTITY);
  const [identityAnchor, setIdentityAnchor] = useState<Anchor | null>(null);
  const [detailAnchor, setDetailAnchor] = useState<Anchor | null>(null);

  function handleOpenDetail(fingerprint: string, anchor: Anchor | null) {
    setDetailAnchor(anchor);
    setDetailFingerprint(fingerprint);
  }

  useEffect(() => {
    // Mica is the lightweight native backdrop for Windows 11. Other platforms
    // keep the existing opaque page background, and Windows chooses its own
    // solid fallback when transparency is disabled or the system requires it.
    if (!navigator.userAgent.includes("Windows")) return;

    let cancelled = false;
    getCurrentWindow()
      .setEffects({ effects: [Effect.Mica] })
      .then(() => {
        if (!cancelled) {
          document.documentElement.classList.add("window-material-active");
        }
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        document.documentElement.classList.remove("window-material-active");
        // Unsupported Windows versions/graphics environments must not prevent
        // startup; the ordinary theme background remains in place.
        console.debug(
          "ClipX: native Mica unavailable; keeping the solid background.",
          error,
        );
      });

    return () => {
      cancelled = true;
      document.documentElement.classList.remove("window-material-active");
    };
  }, []);

  useEffect(() => subscribeToast(setToast), []);

  const [notifBadge, setNotifBadge] = useState({ unread: 0, waiting: 0 });
  useEffect(() => {
    let cancelled = false;
    invoke<{ unread: number; waiting: number }>("nc_badge")
      .then((badge) => {
        if (!cancelled) setNotifBadge(badge);
      })
      .catch(() => {});
    const unlisten = listen<{ unread: number; waiting: number }>(
      "nc-badge",
      ({ payload }) => setNotifBadge(payload),
    );
    return () => {
      cancelled = true;
      void unlisten.then((off) => off());
    };
  }, []);

  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    appWindow
      .onDragDropEvent((event) => {
        if (cancelled || screen !== "sync") return;
        if (event.payload.type === "enter" || event.payload.type === "over") {
          setIsFileDragActive(true);
        } else if (event.payload.type === "leave") {
          setIsFileDragActive(false);
        } else if (event.payload.type === "drop") {
          setIsFileDragActive(false);
          const paths = event.payload.paths ?? [];
          if (!paths.length) return;
          invoke<PickedFile[]>("get_dropped_files", { paths })
            .then((files) => addPickedFiles(files))
            .catch((e) => showToast(`${e}`, { variant: "error" }));
        }
      })
      .then((off) => {
        unlisten = off;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [appWindow, screen]);

  const refreshPaired = useCallback(async () => {
    try {
      const result: PairedDevice[] = await invoke("get_paired_devices");
      setPaired(result);
      setAvailable((prev) => {
        const pairedIds = new Set(result.map((d) => d.fingerprint));
        return prev.filter((d) => !pairedIds.has(d.fingerprint));
      });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    }
  }, []);

  const refreshAvailable = useCallback(async () => {
    try {
      const result: {
        fingerprint: string;
        name: string;
        deviceType: DeviceType;
      }[] = await invoke("get_available_devices");
      setAvailable((prev) => {
        setShowIdentityDisabled(false);
        const requesting = new Set(
          prev
            .filter((d) => d.pairing === "requesting")
            .map((d) => d.fingerprint),
        );
        return result.map((d) => ({
          ...d,
          pairing: requesting.has(d.fingerprint) ? "requesting" : "idle",
        }));
      });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
      setShowIdentityDisabled(true);
    }
  }, []);

  const refreshHistory = useCallback(async () => {
    try {
      const result: ClipItem[] = await invoke("get_clipboard_history", {
        limit: null,
      });
      setHistory(result);
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    }
  }, []);

  const getThisDeviceIdentity = useCallback(async () => {
    try {
      const result: OwnIdentity = await invoke("get_this_device_identity");
      setOwnIdentity(result);
    } catch (e) {
      showToast(`${e}`, {
        variant: "error",
        onOk: () => appWindow.close(),
        okLabel: "Close App",
      });
    }
  }, []);

  useEffect(() => {
    // The main window stays hidden until these first requests settle, so it
    // opens already populated instead of flashing empty lists.
    const initialLoads = [
      getThisDeviceIdentity(),
      refreshPaired(),
      refreshAvailable(),
      refreshHistory(),
    ];

    async function positionAppWindow() {
      // Positioning is best-effort. On Wayland/WSLg there may be no monitor
      // info or setPosition may be refused; that must never keep the window
      // from being shown, or the app is invisible on desktops without a tray.
      try {
        // Only on launch: sessionStorage survives a reload (Ctrl+R, Refresh)
        // but not a restart, so a refresh doesn't snap a moved window back.
        let placed = false;
        try {
          placed = sessionStorage.getItem("clipx-window-placed") === "1";
        } catch {
          // Storage unavailable: treat as a fresh launch and place the window.
        }
        const monitor = placed ? null : await currentMonitor();
        if (monitor) {
          const windowSize = await appWindow.outerSize();
          const workArea = monitor.workArea;
          const margin = 30;
          const x =
            workArea.position.x +
            workArea.size.width -
            windowSize.width -
            margin;
          const y =
            workArea.position.y +
            workArea.size.height -
            windowSize.height -
            margin;
          await appWindow.setPosition(new PhysicalPosition(x, y));
          sessionStorage.setItem("clipx-window-placed", "1");
        }
      } catch (error) {
        console.debug(
          "ClipX: could not position the window; using default placement.",
          error,
        );
      }
      // Hidden only for login launches on systems that have a tray to reopen
      // it from (decided by the core side); a manual launch always shows.
      const hidden = await invoke<boolean>("start_hidden").catch(() => false);
      if (!hidden) {
        // Wait for the first device/history data (capped, so a slow or failed
        // request can never keep the window from opening), then show it.
        await Promise.race([
          Promise.allSettled(initialLoads),
          new Promise<void>((resolve) => window.setTimeout(resolve, 2000)),
        ]);
        await appWindow.show();
      }
    }
    positionAppWindow();

    // Push-based sync: the core emits these whenever paired/connection or
    // clipboard-history state actually changes, so we just re-fetch on
    // demand instead of guessing an interval.
    let cancelled = false;
    const unlistenPromises = [
      listen("devices-changed", () => {
        if (cancelled) return;

        void refreshPaired();
        void refreshAvailable();
      }),
      listen("clipboard-changed", () => {
        if (!cancelled) refreshHistory();
      }),
      listen<{
        entry_id: string;
        file_id: string;
        done: number;
        total: number;
        state: string;
        message: string;
      }>("file-transfer", ({ payload }) => {
        if (cancelled) return;
        setDownloadProgress((prev) => ({
          ...prev,
          [payload.entry_id]: payload,
          [payload.file_id]: payload,
        }));
      }),
      listen("open-diagnostics", () => {
        if (!cancelled) setScreen("diagnostics");
      }),
      // Registered once, for the lifetime of the app — routes by
      // device_id to whichever "Requesting…" button it belongs to.
      listen<PairingEventPayload>("pairing-event", ({ payload }) => {
        if (cancelled) return;
        if (payload.state === PAIRING_STATE.STARTED) return;

        setAvailable((prev) =>
          prev.map((d) =>
            d.fingerprint === payload.device_id ? { ...d, pairing: "idle" } : d,
          ),
        );

        if (payload.state === PAIRING_STATE.FAILED) {
          showToast(payload.message || "Pairing failed", { variant: "error" });
        }
      }),
    ];
    return () => {
      cancelled = true;
      unlistenPromises.forEach((p) => p.then((unlisten) => unlisten()));
    };
  }, []);

  async function handleConnect(fingerprint: string) {
    setPaired((prev) =>
      prev.map((d) =>
        d.fingerprint === fingerprint ? { ...d, connection: "connecting" } : d,
      ),
    );
    try {
      await invoke("connect_device", { deviceId: fingerprint });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    }
    // Do not refresh here: connect_device resolving only means the request
    // was made, not that the connection is up. The devices-changed event
    // will pick up the real state, which gives "connecting" time to show.
  }

  async function handleDisconnect(fingerprint: string) {
    setDetailFingerprint(null);
    try {
      await invoke("disconnect_device", { deviceId: fingerprint });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    } finally {
      refreshPaired();
    }
  }

  async function handleForget(fingerprint: string) {
    setDetailFingerprint(null);
    setPaired((prev) => prev.filter((d) => d.fingerprint !== fingerprint)); // optimistic
    try {
      await invoke("forget_device", { deviceId: fingerprint });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
      refreshPaired(); // roll back the optimistic removal on failure
    }
  }

  async function handleToggleAutoConnect(fingerprint: string, value: boolean) {
    setPaired((prev) =>
      prev.map((d) =>
        d.fingerprint === fingerprint ? { ...d, autoConnect: value } : d,
      ),
    );
    try {
      await invoke("set_auto_connect", {
        deviceId: fingerprint,
        autoConnect: value,
      });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
      refreshPaired();
    }
  }

  async function handlePair(fingerprint: string) {
    setAvailable((prev) =>
      prev.map((d) =>
        d.fingerprint === fingerprint ? { ...d, pairing: "requesting" } : d,
      ),
    );
    try {
      await invoke("pair_device", { deviceId: fingerprint });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
      setAvailable((prev) =>
        prev.map((d) =>
          d.fingerprint === fingerprint ? { ...d, pairing: "idle" } : d,
        ),
      );
      return;
    }

    // Normal resolution comes from the "pairing-event" listener registered
    // once in the top-level useEffect. Core's own pair session TTL is 90s —
    // this is just a last-resort safety net in case an event is ever missed.
    setTimeout(() => {
      setAvailable((prev) =>
        prev.map((d) =>
          d.fingerprint === fingerprint && d.pairing === "requesting"
            ? { ...d, pairing: "idle" }
            : d,
        ),
      );
    }, 95_000);
  }

  // Copy goes through the core (not the webview clipboard) so the write is
  // registered with the echo guard and rich text keeps its HTML.
  function handleCopy(id: string) {
    invoke("copy_clipboard_entry", { id })
      .then(() => showToast("Copied to clipboard"))
      .catch((e) => showToast(`${e}`, { variant: "error" }));
  }

  function handleRemoveHistory(id: string) {
    setHistory((prev) => prev.filter((item) => item.id !== id));
    invoke("remove_clipboard_entry", { id }).catch((e) =>
      showToast(`${e}`, { variant: "error" }),
    );
  }

  function handleClearAll() {
    setHistory([]);
    invoke("clear_clipboard_history").catch((e) =>
      showToast(`${e}`, { variant: "error" }),
    );
  }

  function openTextEditor() {
    const existingText =
      sendContent?.source === "written"
        ? sendContent.text
        : sendContent?.source === "clipboard" &&
            sendContent.clipboard.kind !== "image"
          ? sendContent.clipboard.text
          : "";

    setTextDraft(existingText);
    setEditingText(true);
  }

  function saveWrittenText() {
    if (!textDraft.trim()) {
      showToast("Enter some text before saving");
      return;
    }

    setSendContent({
      source: "written",
      text: textDraft,
    });

    setEditingText(false);
  }

  const connectedCount = paired.filter(
    (d) => d.connection === "connected",
  ).length;

  const refreshCurrentClipboard = useCallback(async () => {
    const clipboard = navigator.clipboard;

    if (!clipboard) {
      showToast("Clipboard access is unavailable", {
        variant: "error",
      });
      return;
    }

    // First try the richer clipboard API.
    if (typeof clipboard.read === "function") {
      try {
        const items = await clipboard.read();
        const item = items[0];

        if (item) {
          // Rich text
          if (item.types.includes("text/html")) {
            const html = await (await item.getType("text/html")).text();

            const text = item.types.includes("text/plain")
              ? await (await item.getType("text/plain")).text()
              : html.replace(/<[^>]*>/g, "");

            setSendContent({
              source: "clipboard",
              clipboard: {
                kind: "rich_text",
                text,
                html,
              },
            });

            return;
          }

          // Image
          const imageType = item.types.find((type) =>
            type.startsWith("image/"),
          );

          if (imageType) {
            const blob = await item.getType(imageType);
            const bitmap = await createImageBitmap(blob);

            try {
              const canvas = document.createElement("canvas");
              canvas.width = bitmap.width;
              canvas.height = bitmap.height;

              const ctx = canvas.getContext("2d");

              if (ctx) {
                ctx.drawImage(bitmap, 0, 0);

                const pixels = ctx.getImageData(
                  0,
                  0,
                  bitmap.width,
                  bitmap.height,
                ).data;

                setSendContent({
                  source: "clipboard",
                  clipboard: {
                    kind: "image",
                    text: `Image ${bitmap.width}×${bitmap.height}`,
                    width: bitmap.width,
                    height: bitmap.height,
                    rgba: Array.from(pixels),
                  },
                });

                return;
              }
            } finally {
              bitmap.close();
            }
          }
        }
      } catch (error) {
        // Rich clipboard access failed.
        // Continue and try the simpler text API below.
        console.debug(
          "ClipX: rich clipboard read failed; trying readText().",
          error,
        );
      }
    }

    // Fallback: plain text clipboard access.
    try {
      const text = await clipboard.readText();

      if (!text) {
        showToast("The system clipboard is empty");
        return;
      }

      setSendContent({
        source: "clipboard",
        clipboard: {
          kind: "text",
          text,
        },
      });
    } catch (error) {
      showToast(`Could not read clipboard: ${String(error)}`, {
        variant: "error",
      });
    }
  }, []);

  async function addPickedFiles(files: PickedFile[]) {
    const pool = [...outgoingRef.current];
    const accepted: PickedFile[] = [];
    let skipped = 0;
    for (const file of files) {
      let dup = pool.some((e) => e.path === file.path);
      if (!dup) {
        // Cheap filter first: only compare bytes against same-size files.
        for (const e of pool.filter((e) => e.size === file.size)) {
          const same = await invoke<boolean>("files_identical", {
            a: e.path,
            b: file.path,
          }).catch(() => false);
          if (same) {
            dup = true;
            break;
          }
        }
      }
      if (dup) skipped++;
      else {
        pool.push(file);
        accepted.push(file);
      }
    }
    if (accepted.length) {
      setOutgoingFiles((prev) => [
        ...prev,
        ...accepted.filter((f) => !prev.some((p) => p.path === f.path)),
      ]);
    }
    if (skipped)
      showToast(`${skipped} duplicate file${skipped === 1 ? "" : "s"} skipped`);
  }

  async function pickFiles() {
    try {
      const files = await invoke<PickedFile[]>("pick_files");
      addPickedFiles(files);
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    }
  }

  const sendReadyItems = useCallback(async () => {
    if (connectedCount === 0) {
      showToast("No connected devices", { variant: "error" });
      return;
    }

    const hasContent = !!sendContent;

    if (!hasContent && outgoingFiles.length === 0) {
      showToast("Nothing ready to send", { variant: "error" });
      return;
    }

    setSendingReadyItems(true);

    try {
      if (sendContent) {
        if (sendContent.source === "written") {
          await invoke("send_text", {
            content: sendContent.text,
          });
        } else {
          const item = sendContent.clipboard;

          if (item.kind === "rich_text") {
            await invoke("send_rich_text", {
              text: item.text,
              html: item.html || item.text,
            });
          } else if (
            item.kind === "image" &&
            item.rgba &&
            item.width &&
            item.height
          ) {
            await invoke("send_image", {
              width: item.width,
              height: item.height,
              rgba: item.rgba,
            });
          } else {
            await invoke("send_text", {
              content: item.text,
            });
          }
        }
      }

      for (const file of outgoingFiles) {
        await invoke("send_file_path", {
          name: file.name,
          mimeType: file.mimeType,
          path: file.path,
        });
      }

      setOutgoingFiles([]);

      showToast(
        `${(hasContent ? 1 : 0) + outgoingFiles.length} item(s) offered`,
      );
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    } finally {
      setSendingReadyItems(false);
    }
  }, [connectedCount, sendContent, outgoingFiles]);

  async function handleDownloadHistoryFile(id: string) {
    try {
      await invoke("download_clipboard_file", { id });
      showToast("File download requested");
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    }
  }

  async function handleRevealHistoryFile(path: string) {
    try {
      await invoke("reveal_file_location", { path });
    } catch (e) {
      showToast(`${e}`, { variant: "error" });
    }
  }

  const detailDevice =
    paired.find((d) => d.fingerprint === detailFingerprint) ?? null;

  const refreshDiagnostics = useCallback(async () => {
    try {
      setDiagnostics(
        await invoke<DiagnosticSnapshot>("get_diagnostic_snapshot"),
      );
      setDiagnosticsError(null);
    } catch (error) {
      setDiagnosticsError(String(error));
    } finally {
      setDiagnosticsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (screen !== "diagnostics") return;
    setDiagnosticsLoading(true);
    void refreshDiagnostics();
    const timer = window.setInterval(() => {
      void refreshDiagnostics();
    }, 2000);
    return () => window.clearInterval(timer);
  }, [screen, refreshDiagnostics]);

  async function handleOpenLogsFolder() {
    try {
      const path = await invoke<string>("open_logs_folder");
      showToast(`Opened logs folder: ${path}`);
    } catch (error) {
      showToast(String(error), { variant: "error" });
    }
  }

  async function handleCopyDiagnosticReport() {
    const reportLogs = visibleDiagnosticLogs(
      diagnostics,
      diagnosticsClearMarker,
    );
    const report = [
      "ClipX diagnostic report",
      `Application version: ${diagnostics?.appVersion ?? "unknown"}`,
      `Platform: ${navigator.userAgent}`,
      `Core status: ${diagnostics?.coreStatus ?? "unknown"}`,
      `Log directory: ${diagnostics?.logDirectory ?? "unknown"}`,
      `Latest log file: ${diagnostics?.latestLogFile ?? "none"}`,
      "",
      ...reportLogs,
    ].join("\n");
    try {
      if (!navigator.clipboard)
        throw new Error("Clipboard access is unavailable");
      await navigator.clipboard.writeText(report);
      showToast("Diagnostic report copied. Review it before sharing.");
    } catch (error) {
      showToast(String(error), { variant: "error" });
    }
  }

  function handleClearDiagnosticView() {
    const logs = diagnostics?.recentLogs ?? [];
    setDiagnosticsClearMarker(logs.length ? logs[logs.length - 1] : null);
    showToast(
      "Cleared the visible log view. Saved log files were not changed.",
    );
  }

  function renderDevicesScreen() {
    return (
      <ScrollFade className="devices-panel-scroll">
        <div className="devices-grid">
          <section className="device-section">
            <div className="section-header">
              <h2 className="section-title">Paired Devices</h2>
            </div>
            <CollapsibleList
              items={paired}
              keyOf={(d) => d.fingerprint}
              renderItem={(d) => (
                <PairedRow
                  device={d}
                  onConnect={handleConnect}
                  onOpenDetail={handleOpenDetail}
                />
              )}
              emptyTitle="No paired devices"
              emptyHint="Pair a nearby device to sync your clipboard."
            />
          </section>

          <section className="device-section">
            <div className="section-header">
              <h2 className="section-title">Nearby Devices</h2>
            </div>
            {available.length === 0 ? (
              <EmptyState
                title="No nearby devices yet"
                hint={"Discovered devices will appear here automatically."}
              />
            ) : (
              <ScrollFade className="devices-scroll">
                <div className="device-list">
                  {available.map((d) => (
                    <AvailableRow
                      key={d.fingerprint}
                      device={d}
                      onPair={handlePair}
                    />
                  ))}
                </div>
              </ScrollFade>
            )}
          </section>
        </div>
      </ScrollFade>
    );
  }

  function renderSyncScreen() {
    const readyCount = (sendContent ? 1 : 0) + outgoingFiles.length;

    const contentKindLabel = !sendContent
      ? null
      : sendContent.source === "written"
        ? "Written text"
        : sendContent.clipboard.kind === "image"
          ? "Image"
          : sendContent.clipboard.kind === "rich_text"
            ? "Rich text"
            : "Text";

    return (
      <section
        className={`sync-screen ${isFileDragActive ? "sync-screen--dragging" : ""}`}
      >
        <ScrollFade className="sync-scroll">
          <div className="sync-shell">
            <header className="sync-page-header">
              <div>
                <div className="sync-heading-row">
                  <div className="sync-device-transfer-icon" aria-hidden="true">
                    <Send size={20} />
                  </div>
                  <div
                    style={{
                      flex: "1",
                      display: "flex",
                      justifyContent: "space-between",
                      alignItems: "center",
                    }}
                  >
                    <div className="sync-heading">Send</div>
                    <div
                      className={`sync-connection-pill ${connectedCount > 0 ? "online" : "offline"}`}
                    >
                      <span className="status-dot" />
                      {connectedCount > 0
                        ? `${connectedCount} connected`
                        : "No device connected"}
                    </div>
                  </div>
                </div>
              </div>
            </header>

            <div className="sync-layout">
              <div className="sync-primary-column">
                <section className="sync-panel clipboard-panel">
                  <div className="sync-panel-header sync-clipboard-header">
                    <div className="sync-panel-title">Content to send</div>

                    <div className="sync-clipboard-actions">
                      {contentKindLabel && (
                        <span className="sync-kind ready">
                          {contentKindLabel}
                        </span>
                      )}

                      <button
                        className="sync-icon-action word-button"
                        onClick={refreshCurrentClipboard}
                        title="Paste from system clipboard"
                        aria-label="Paste from system clipboard"
                        disabled={editingText || sendingReadyItems}
                      >
                        <ClipboardPaste size={16} />
                      </button>

                      <button
                        className="sync-icon-action word-button"
                        onClick={openTextEditor}
                        title="Write your own text"
                        aria-label="Write your own text"
                        disabled={editingText || sendingReadyItems}
                      >
                        <PencilLine size={16} />
                      </button>
                    </div>
                  </div>

                  {editingText ? (
                    <div className="sync-message-editor-wrap">
                      <ScrollFadeTextarea
                        className="sync-message-editor"
                        value={textDraft}
                        onChange={(e) => setTextDraft(e.target.value)}
                        placeholder="Write a message or paste text here..."
                        rows={5}
                        autoFocus
                        aria-label="Message to send"
                      />

                      <div className="sync-message-editor-footer">
                        <button
                          className="sync-editor-cancel"
                          onClick={() => setEditingText(false)}
                        >
                          Cancel
                        </button>

                        <button
                          className="sync-editor-save"
                          onClick={saveWrittenText}
                          disabled={!textDraft.trim()}
                        >
                          Save text
                        </button>
                      </div>
                    </div>
                  ) : sendContent ? (
                    <div className="sync-clipboard-preview">
                      <div className="sync-preview-icon">
                        {sendContent.source === "written" ? (
                          <Type size={16} />
                        ) : sendContent.clipboard.kind === "image" ? (
                          <ImageIcon size={16} />
                        ) : sendContent.clipboard.kind === "rich_text" ? (
                          <FileText size={16} />
                        ) : (
                          <Type size={16} />
                        )}
                      </div>

                      <div className="sync-preview-copy">
                        <div className="sync-preview-text">
                          {sendContent.source === "written"
                            ? sendContent.text
                            : sendContent.clipboard.text}
                        </div>

                        <div className="sync-preview-meta">
                          {sendContent.source === "written"
                            ? "Written text"
                            : "From system clipboard"}
                        </div>
                      </div>

                      <button
                        className="sync-selection-remove"
                        title="Remove content"
                        aria-label="Remove content from send queue"
                        onClick={() => setSendContent(null)}
                      >
                        <X size={14} />
                      </button>
                    </div>
                  ) : (
                    <button
                      type="button"
                      className="sync-clipboard-empty"
                      onClick={refreshCurrentClipboard}
                      title="Paste from system clipboard"
                    >
                      <div className="sync-empty-glyph">
                        <ClipboardPaste size={20} />
                      </div>

                      <div>
                        <div className="sync-empty-title">Nothing selected</div>

                        <div className="sync-empty-copy">
                          Paste something from your clipboard or write your own
                          message.
                        </div>
                      </div>
                    </button>
                  )}
                </section>

                <section className="sync-panel files-panel">
                  <div className="sync-panel-header">
                    <div>
                      <div className="sync-panel-title">Files to send</div>
                      <div className="sync-panel-subtitle">
                        Files stay local until you explicitly press Send.
                      </div>
                    </div>
                    {outgoingFiles.length > 0 && (
                      <div className="sync-file-extras">
                        <button className="word-button" onClick={() => {
                          setOutgoingFiles([])
                        }}>Clear all</button>
                        <span className="sync-count-chip">
                          {outgoingFiles.length}
                        </span>
                      </div>
                    )}
                  </div>

                  <button
                    className={`sync-dropzone ${isFileDragActive ? "dragging" : ""}`}
                    onClick={pickFiles}
                    title="Choose files"
                  >
                    <div className="sync-drop-icon">
                      <Upload size={22} />
                    </div>
                    <div className="sync-drop-title">Drop files anywhere</div>
                    <div className="sync-drop-copy">
                      or click to choose files
                    </div>
                  </button>

                  {outgoingFiles.length > 0 && (
                    <div className="sync-file-list">
                      {outgoingFiles.map((file, index) => (
                        <div
                          className="sync-file-row"
                          key={`${file.name}:${file.size}:${index}`}
                        >
                          <div className="sync-file-icon">
                            {file.mimeType.startsWith("image/") ? (
                              <ImageIcon size={16} />
                            ) : (
                              <FileIcon size={16} />
                            )}
                          </div>
                          <div className="sync-file-info">
                            <div className="sync-file-name">{file.name}</div>
                            <div className="sync-file-meta">
                              {file.mimeType} · {formatDataSize(file.size)}
                            </div>
                          </div>
                          <button
                            className="sync-file-remove"
                            title={`Remove ${file.name}`}
                            onClick={() =>
                              setOutgoingFiles((prev) =>
                                prev.filter((_, i) => i !== index),
                              )
                            }
                          >
                            <X size={14} />
                          </button>
                        </div>
                      ))}
                    </div>
                  )}
                </section>
              </div>

              <aside className="sync-aside">
                <div className="sync-summary-panel">
                  <div className="sync-summary-label">READY TO SEND</div>
                  <div className="sync-summary-number">{readyCount}</div>
                  <div className="sync-summary-caption">
                    {readyCount === 1 ? "item" : "items"} in this send
                  </div>

                  <div className="sync-summary-breakdown">
                    {sendContent && (
                      <div className="sync-breakdown-row">
                        <span>
                          {sendContent.source === "written"
                            ? "Written text"
                            : "Clipboard"}
                        </span>
                        <strong>1</strong>
                      </div>
                    )}
                    {outgoingFiles.length > 0 && (
                      <div className="sync-breakdown-row">
                        <span>Files</span>
                        <strong>{outgoingFiles.length}</strong>
                      </div>
                    )}
                    {!sendContent && outgoingFiles.length === 0 && (
                      <div className="sync-breakdown-empty">
                        Add something to the queue to enable sending.
                      </div>
                    )}
                  </div>

                  <div className="sync-summary-destination">
                    <span
                      className={`status-dot ${connectedCount > 0 ? "connected" : "disconnected"}`}
                    />
                    <div>
                      <div>
                        {connectedCount > 0
                          ? "Connected devices"
                          : "No connected devices"}
                      </div>
                      <small>
                        {connectedCount > 0
                          ? "The bundle will be sent to connected peers."
                          : "Connect a paired device first."}
                      </small>
                    </div>
                  </div>

                  <button
                    className="sync-send-button"
                    onClick={sendReadyItems}
                    disabled={
                      sendingReadyItems ||
                      connectedCount === 0 ||
                      readyCount === 0
                    }
                  >
                    <span>{sendingReadyItems ? "Preparing…" : "Send now"}</span>
                    <Send size={15} className="sync-send-arrow" />
                  </button>

                  <div className="sync-summary-note">
                    Files are offered as downloadable content and remain
                    available for up to 24 hours.
                  </div>
                </div>
              </aside>
            </div>
          </div>
        </ScrollFade>
      </section>
    );
  }

  function renderDiagnosticsScreen() {
    const visibleLogs = visibleDiagnosticLogs(
      diagnostics,
      diagnosticsClearMarker,
    );
    const visibleLogText = visibleLogs.length
      ? visibleLogs.join("\n")
      : "No visible log entries. New events will appear here.";

    return (
      <section className="diagnostics-screen">
        <div className="diagnostics-heading">
          <div>
            <h2>Help & diagnostics</h2>
            <p>
              Inspect recent core logs. Clear view hides entries here only;
              saved log files are kept.
            </p>
          </div>
          <div className="diagnostics-actions">
            <button
              className="diagnostics-action-button"
              onClick={() => {
                setDiagnosticsLoading(true);
                void refreshDiagnostics();
              }}
              disabled={diagnosticsLoading}
              title="Refresh recent logs"
              aria-label="Refresh recent logs"
            >
              <RefreshCw size={16} aria-hidden="true" />
            </button>
            <button
              className="diagnostics-action-button"
              onClick={handleOpenLogsFolder}
              title="Open the folder containing saved log files"
              aria-label="Open logs folder"
            >
              <FolderOpen size={16} aria-hidden="true" />
            </button>
            <button
              className="diagnostics-action-button"
              onClick={handleCopyDiagnosticReport}
              disabled={!diagnostics}
              title="Copy diagnostic report to clipboard"
              aria-label="Copy diagnostic report"
            >
              <Copy size={16} aria-hidden="true" />
            </button>
            <button
              className="diagnostics-action-button"
              onClick={handleClearDiagnosticView}
              disabled={visibleLogs.length === 0}
              title="Clear visible log entries (saved log files are kept)"
              aria-label="Clear visible log entries without deleting saved log files"
            >
              <Trash2 size={16} aria-hidden="true" />
            </button>
          </div>
        </div>
        <div className="diagnostics-summary">
          <div className="diagnostics-card">
            <span>Application version</span>
            <strong>{diagnostics?.appVersion ?? "Checking…"}</strong>
          </div>
          <div className="diagnostics-card">
            <span>Core status</span>
            <strong
              className={`diagnostics-status diagnostics-status--${diagnostics?.coreStatus ?? "unknown"}`}
            >
              {diagnostics?.coreStatus ?? "Checking…"}
            </strong>
          </div>
          <div className="diagnostics-card">
            <span>Log location</span>
            <strong className="diagnostics-path">
              {diagnostics?.logDirectory ?? "Loading…"}
            </strong>
          </div>
        </div>
        <section className="diagnostics-log-panel">
          <div className="diagnostics-log-heading">
            <h3>Recent logs</h3>
            <span>Live · refreshes every 2 seconds</span>
          </div>
          {diagnosticsError && (
            <p className="diagnostics-error">{diagnosticsError}</p>
          )}
          <ScrollFade
            className="diagnostics-log-scroll"
            autoScrollKey={visibleLogText}
          >
            <pre className="diagnostics-log">{visibleLogText}</pre>
          </ScrollFade>
        </section>
        <p className="diagnostics-privacy">
          Diagnostic logs can contain technical details such as local paths,
          device names, or network information. Review the report before sharing
          it.
        </p>
      </section>
    );
  }

  function renderHistoryScreen() {
    return (
      <section className="history-screen">
        <div className="history-content">
          <div className="history-header">
            <div className="history-title-group">
              <h2 className="history-title">Clipboard History</h2>
              {history.length > 0 && (
                <span className="history-count">{history.length}</span>
              )}
            </div>
            {history.length > 0 && (
              <button className="word-button" onClick={handleClearAll}>
                Clear all
              </button>
            )}
          </div>
          {history.length === 0 ? (
            <EmptyState
              title="Nothing here yet"
              hint="Items received from other devices will show up here."
            />
          ) : (
            <ScrollFade className="history-scroll">
              <div className="device-list">
                {history.map((item) => (
                  <ClipRow
                    key={item.id}
                    item={item}
                    onCopy={handleCopy}
                    onRemove={handleRemoveHistory}
                    onDownload={handleDownloadHistoryFile}
                    onReveal={handleRevealHistoryFile}
                    transfer={
                      downloadProgress[item.id] ??
                      (item.fileId ? downloadProgress[item.fileId] : undefined)
                    }
                    nowMs={nowMs}
                  />
                ))}
              </div>
            </ScrollFade>
          )}
        </div>
      </section>
    );
  }

  return (
    <>
      <header className="title-bar" data-tauri-drag-region>
        <div className="app-logo">
          <img src="/clipx-icon.png" alt="clipx logo" />
          <span className="titlebar-divider" />
          <div className="title-right-controls">
            <button
              className="icon-button titlebar-icon"
              title="settings"
              onClick={() => {
                showToast("This feature is not implemented yet.");
              }}
            >
              <Settings size={15} />
            </button>
            <button
              className={`icon-button titlebar-icon${notifBadge.unread > 0 ? " has-badge" : ""}`}
              title={
                notifBadge.unread > 0
                  ? `Notifications (${notifBadge.unread} unread)`
                  : "Notifications"
              }
              onClick={() => {
                invoke("open_notification_center").catch((error) =>
                  showToast(String(error), { variant: "error" }),
                );
              }}
            >
              <Bell size={15} />
              {notifBadge.unread > 0 && (
                <span
                  className={`titlebar-badge${notifBadge.waiting > 0 ? " urgent" : ""}`}
                >
                  {notifBadge.unread > 9 ? "9+" : notifBadge.unread}
                </span>
              )}
            </button>
          </div>
        </div>
        <div className="window-controls">
          <button
            className="icon-button titlebar-icon"
            title={
              screen === "devices" ? "Clipboard history" : "Back to devices"
            }
            onClick={() =>
              setScreen(screen === "devices" ? "history" : "devices")
            }
          >
            {screen === "devices" ? (
              <History size={15} />
            ) : (
              <ArrowLeft size={15} />
            )}
          </button>

          <button
            className="icon-button titlebar-icon"
            title="Send"
            onClick={() => setScreen("sync")}
          >
            <Send size={15} />
          </button>

          <span className="titlebar-divider" />

          <button
            className="icon-button titlebar-icon"
            title="Help & diagnostics"
            onClick={() => setScreen("diagnostics")}
          >
            <CircleHelp size={15} />
          </button>

          <button
            className={`icon-button titlebar-icon ${showIdentityDisabled ? "button-disable" : ""}`}
            title="Device identity"
            onClick={(e) => {
              const anchor = anchorOf(e.currentTarget);
              getThisDeviceIdentity().finally(() => {
                if (showIdentityDisabled) return;
                setIdentityAnchor(anchor);
                setShowIdentity(true);
              });
            }}
          >
            <MonitorSmartphone size={15} />
          </button>

          <button className="closing-button" onClick={() => appWindow.hide()}>
            <X size={16} />
          </button>
        </div>
      </header>

      <main>
        {screen === "devices"
          ? renderDevicesScreen()
          : screen === "sync"
            ? renderSyncScreen()
            : screen === "history"
              ? renderHistoryScreen()
              : renderDiagnosticsScreen()}
      </main>

      {showIdentity && (
        <DeviceIdentity
          identity={ownIdentity}
          anchor={identityAnchor}
          onClose={() => setShowIdentity(false)}
        />
      )}

      {detailDevice && (
        <DeviceDetail
          device={detailDevice}
          anchor={detailAnchor}
          onClose={() => setDetailFingerprint(null)}
          onDisconnect={handleDisconnect}
          onForget={handleForget}
          onToggleAutoConnect={handleToggleAutoConnect}
        />
      )}

      <Toast toast={toast} />
    </>
  );
}

export default App;
