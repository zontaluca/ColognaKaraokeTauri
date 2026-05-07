import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

const CloudContext = createContext({ syncEvents: {} });

export function CloudProvider({ children, onSyncDone }) {
  // song_dir → latest CloudSyncEvent
  const [syncEvents, setSyncEvents] = useState({});

  const handleSyncDone = useCallback(
    (songDir) => {
      if (onSyncDone) onSyncDone(songDir);
    },
    [onSyncDone]
  );

  useEffect(() => {
    let unlisten;
    (async () => {
      unlisten = await listen("karaoke://cloud-sync", (ev) => {
        const event = ev.payload;
        if (!event) return;
        setSyncEvents((prev) => ({ ...prev, [event.song_dir]: event }));
        if (event.status === "done") handleSyncDone(event.song_dir);
      });
    })();
    return () => unlisten && unlisten();
  }, [handleSyncDone]);

  return (
    <CloudContext.Provider value={{ syncEvents }}>
      {children}
    </CloudContext.Provider>
  );
}

export function useCloud() {
  return useContext(CloudContext);
}

export function CloudToast() {
  const { syncEvents } = useCloud();
  const active = Object.values(syncEvents).find((e) => e.status === "active");
  if (!active) return null;
  return (
    <div className="jobs-toast" style={{ bottom: 88 }}>
      <div className="title">Cloud sync</div>
      <div className="msg">{active.message}</div>
      <div className="progress-track">
        <div
          className="progress-fill"
          style={{ width: `${Math.min(100, (active.progress || 0) * 100)}%` }}
        />
      </div>
    </div>
  );
}
