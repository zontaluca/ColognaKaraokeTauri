import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriEvent } from "./hooks/useTauriEvent.js";

const JobsContext = createContext({
  jobs: [],
  enqueue: async () => null,
  cancel: async () => {},
  onJobDone: null,
});

export function JobsProvider({ children, onJobDone }) {
  const [jobs, setJobs] = useState([]);
  // Last known status per job id: onJobDone fires only on the transition to
  // "done", not for every event that repeats the final state.
  const statusRef = useRef(new Map());

  useEffect(() => {
    invoke("jobs_list")
      .then((initial) => {
        if (!Array.isArray(initial)) return;
        for (const j of initial) statusRef.current.set(j.id, j.status);
        setJobs(initial);
      })
      .catch(() => {});
  }, []);

  useTauriEvent("karaoke://jobs", (ev) => {
    const job = ev.payload;
    if (!job || !job.id) return;
    setJobs((prev) => {
      const i = prev.findIndex((j) => j.id === job.id);
      if (i >= 0) {
        const next = [...prev];
        next[i] = job;
        return next;
      }
      return [...prev, job];
    });
    const prevStatus = statusRef.current.get(job.id);
    statusRef.current.set(job.id, job.status);
    if (job.status === "done" && prevStatus !== "done" && onJobDone) onJobDone(job);
  });

  useTauriEvent("karaoke://jobs-list", (ev) => {
    if (!Array.isArray(ev.payload)) return;
    for (const j of ev.payload) statusRef.current.set(j.id, j.status);
    setJobs(ev.payload);
  });

  const enqueue = useCallback(async (url) => {
    return await invoke("jobs_enqueue", { url });
  }, []);
  const cancel = useCallback(async (id) => {
    return await invoke("jobs_cancel", { id });
  }, []);

  const value = useMemo(() => ({ jobs, enqueue, cancel }), [jobs, enqueue, cancel]);

  return (
    <JobsContext.Provider value={value}>
      {children}
    </JobsContext.Provider>
  );
}

export function useJobs() {
  return useContext(JobsContext);
}

export function JobsToast() {
  const { jobs } = useJobs();
  const active = jobs.find((j) => j.status === "active" || j.status === "queued");
  if (!active) return null;
  return (
    <div className="jobs-toast">
      <div className="title">
        Processing {active.status === "queued" ? "(queued)" : `step ${active.current_step + 1}`}
      </div>
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
