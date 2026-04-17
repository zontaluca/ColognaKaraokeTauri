import { createContext, useContext, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const JobsContext = createContext({
  jobs: [],
  enqueue: async () => null,
  cancel: async () => {},
  onJobDone: null,
});

export function JobsProvider({ children, onJobDone }) {
  const [jobs, setJobs] = useState([]);

  useEffect(() => {
    let unlistenJob, unlistenList;
    (async () => {
      try {
        const initial = await invoke("jobs_list");
        if (Array.isArray(initial)) setJobs(initial);
      } catch {}
      unlistenJob = await listen("karaoke://jobs", (ev) => {
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
        if (job.status === "done" && onJobDone) onJobDone(job);
      });
      unlistenList = await listen("karaoke://jobs-list", (ev) => {
        if (Array.isArray(ev.payload)) setJobs(ev.payload);
      });
    })();
    return () => {
      unlistenJob && unlistenJob();
      unlistenList && unlistenList();
    };
  }, [onJobDone]);

  const enqueue = async (url) => {
    return await invoke("jobs_enqueue", { url });
  };
  const cancel = async (id) => {
    return await invoke("jobs_cancel", { id });
  };

  return (
    <JobsContext.Provider value={{ jobs, enqueue, cancel }}>
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
