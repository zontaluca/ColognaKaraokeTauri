import { createContext, useContext, useMemo, useState } from "react";
import { useTauriEvent } from "./hooks/useTauriEvent.js";

const CloudContext = createContext({ syncEvents: {} });

export function CloudProvider({ children, onSyncDone }) {
  // song_dir → latest CloudSyncEvent
  const [syncEvents, setSyncEvents] = useState({});

  useTauriEvent("karaoke://cloud-sync", (ev) => {
    const event = ev.payload;
    if (!event) return;
    setSyncEvents((prev) => ({ ...prev, [event.song_dir]: event }));
    if (event.status === "done" && onSyncDone) onSyncDone(event.song_dir);
  });

  const value = useMemo(() => ({ syncEvents }), [syncEvents]);

  return (
    <CloudContext.Provider value={value}>
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
