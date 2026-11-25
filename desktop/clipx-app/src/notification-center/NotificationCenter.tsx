import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { X } from "lucide-react";
import "../styles/index.css";
import { EmptyState } from "../components/DeviceParts";
import { ScrollFade } from "../components/ScrollFade";
import { useWindowMaterial } from "../useWindowMaterial";
import { NotificationCard } from "./NotificationCards";
import { canMarkRead, isWaitingPrompt, splitByTab } from "./logic";
import type {
  AppNotification,
  PromptDecision,
  TransferAction,
} from "./types";
import "./nc.css";

type Tab = "unread" | "read";

/** How long a refused action's message stays on its card. */
const ERROR_MS = 7000;

/** Ticks once a second while `active`, for countdowns. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
}

export default function NotificationCenter() {
  useWindowMaterial();
  const [items, setItems] = useState<AppNotification[]>([]);
  const [tab, setTab] = useState<Tab>("unread");
  const [errors, setErrors] = useState<Record<string, string>>({});
  const errorTimers = useRef<Record<string, number>>({});

  const refresh = useCallback(async () => {
    try {
      setItems(await invoke<AppNotification[]>("nc_list"));
    } catch (error) {
      console.error("could not load notifications", error);
    }
  }, []);

  useEffect(() => {
    // Transfers that began before this window existed, then the list itself.
    void invoke("nc_refresh_transfers")
      .catch(() => {})
      .finally(refresh);
    const unlisten = listen("nc-changed", () => void refresh());
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    return () => {
      void unlisten.then((fn) => fn());
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh]);

  const { unread, read } = useMemo(() => splitByTab(items), [items]);
  const waiting = useMemo(() => items.filter(isWaitingPrompt).length, [items]);
  const now = useNow(waiting > 0);

  useEffect(() => {
    document.title = unread.length > 0
      ? `Clipx notifications (${unread.length})`
      : "Clipx notifications";
  }, [unread.length]);

  const showError = useCallback((id: string, message: string) => {
    setErrors((e) => ({ ...e, [id]: message }));
    window.clearTimeout(errorTimers.current[id]);
    errorTimers.current[id] = window.setTimeout(() => {
      setErrors((e) => {
        const { [id]: _gone, ...rest } = e;
        return rest;
      });
    }, ERROR_MS);
  }, []);

  const run = useCallback(
    async (id: string, command: string, args: Record<string, unknown>) => {
      try {
        await invoke(command, args);
      } catch (error) {
        showError(id, String(error));
      } finally {
        void refresh();
      }
    },
    [refresh, showError],
  );

  const handlers = useMemo(
    () => ({
      onResolve: (id: string, decision: PromptDecision) =>
        void run(id, "nc_resolve", { id, decision }),
      onTransfer: (id: string, action: TransferAction) =>
        void run(id, "nc_transfer_action", { id, action }),
      onMarkRead: (id: string) => void run(id, "nc_mark_read", { id }),
      onRemove: (id: string) => void run(id, "nc_remove", { id }),
      onReveal: (id: string) => void run(id, "nc_reveal_download", { id }),
    }),
    [run],
  );

  const shown = tab === "unread" ? unread : read;

  return (
    <>
      <header className="title-bar" data-tauri-drag-region>
        <div className="app-logo">
          <img src="/clipx-icon.png" alt="clipx logo" />
          <span className="titlebar-divider" />
          <span className="nc-title-text">Notifications</span>
        </div>
        <div className="window-controls">
          <button
            className="closing-button"
            title="Close"
            onClick={() => void getCurrentWindow().hide()}
          >
            <X size={16} />
          </button>
        </div>
      </header>

      <main>
        <section className="history-screen">
          <div className="history-content">
            <div className="history-header">
              <div className="history-title-group">
                <h2 className="history-title">Notifications</h2>
                {unread.length > 0 && (
                  <span className="history-count">{unread.length}</span>
                )}
              </div>
              {tab === "unread" ? (
                <button
                  className="word-button nc-header-action"
                  disabled={!unread.some(canMarkRead)}
                  onClick={() => void run("", "nc_mark_all_read", {})}
                >
                  Mark all read
                </button>
              ) : (
                <button
                  className="word-button nc-header-action"
                  disabled={read.length === 0}
                  onClick={() => void run("", "nc_clear_read", {})}
                >
                  Clear read
                </button>
              )}
            </div>

            <nav className="nc-tabs" role="tablist" data-tab={tab}>
              <span className="nc-tab-pill" aria-hidden="true" />
              <button
                role="tab"
                aria-selected={tab === "unread"}
                className="nc-tab"
                onClick={() => setTab("unread")}
              >
                Unread
                {unread.length > 0 && (
                  <span className="history-count">{unread.length}</span>
                )}
              </button>
              <button
                role="tab"
                aria-selected={tab === "read"}
                className="nc-tab"
                onClick={() => setTab("read")}
              >
                Read
                {read.length > 0 && (
                  <span className="history-count nc-count-quiet">
                    {read.length}
                  </span>
                )}
              </button>
            </nav>

            {/* Remounted per tab: resets the scroll and plays the slide-in. */}
            <div key={tab} className={`nc-pane nc-enter-${tab}`}>
              {shown.length === 0 ? (
                <EmptyState
                  title={
                    tab === "unread"
                      ? "Nothing needs you right now"
                      : "Nothing read yet"
                  }
                  hint={
                    tab === "unread"
                      ? "Requests and transfers will show up here."
                      : "Finished notifications stay here until the app closes."
                  }
                />
              ) : (
                <ScrollFade className="history-scroll">
                  <div className="device-list" role="tabpanel">
                    {shown.map((n) => (
                      <NotificationCard
                        key={n.id}
                        n={n}
                        now={now}
                        error={errors[n.id]}
                        {...handlers}
                      />
                    ))}
                  </div>
                </ScrollFade>
              )}
            </div>
          </div>
        </section>
      </main>
    </>
  );
}
