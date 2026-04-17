import { useState } from "react";
import { useJobs } from "../jobsContext.jsx";

const STEPS = [
  "Download audio",
  "Fetch lyrics",
  "Fetch album art",
  "Separate vocals",
  "Align words",
  "Compute pitch",
  "Save",
];

function Step({ label, status }) {
  return (
    <div className={`step ${status}`}>
      <span className="step-dot" />
      <span>{label}</span>
    </div>
  );
}

export default function Download() {
  const { jobs, enqueue, cancel } = useJobs();
  const [url, setUrl] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [lastJobId, setLastJobId] = useState(null);

  const job = jobs.find((j) => j.id === lastJobId);

  async function start() {
    if (!url.trim() || submitting) return;
    setSubmitting(true);
    try {
      const id = await enqueue(url.trim());
      setLastJobId(id);
      setUrl("");
    } catch (e) {
      alert(`Enqueue failed: ${e}`);
    } finally {
      setSubmitting(false);
    }
  }

  const statusFor = (i) => {
    if (!job) return "pending";
    if (job.status === "error" && job.current_step === i) return "error";
    if (job.current_step > i) return "done";
    if (job.current_step === i)
      return job.status === "active" ? "active" : job.status === "done" ? "done" : "active";
    return "pending";
  };

  return (
    <>
      <div className="main-header">
        <div>
          <h2>Download</h2>
          <p>Queue YouTube links — jobs run in background.</p>
        </div>
      </div>

      <div className="content stack">
        <div className="card">
          <label>YouTube URL</label>
          <div className="row">
            <input
              className="input"
              placeholder="https://www.youtube.com/watch?v=..."
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              disabled={submitting}
            />
            <button
              className="btn btn-primary"
              onClick={start}
              disabled={submitting || !url.trim()}
            >
              {submitting ? "Queuing…" : "🚀 Queue"}
            </button>
          </div>
        </div>

        {job && (
          <div className="card">
            <div className="row spread" style={{ marginBottom: 8 }}>
              <h3>Current job</h3>
              {job.status === "active" || job.status === "queued" ? (
                <button
                  className="btn btn-secondary"
                  style={{ fontSize: 12, padding: "4px 10px" }}
                  onClick={() => cancel(job.id)}
                >
                  Cancel
                </button>
              ) : null}
            </div>
            <div className="status-text">{job.message}</div>
            <div className="progress-track">
              <div
                className="progress-fill"
                style={{ width: `${Math.min(100, (job.progress || 0) * 100)}%` }}
              />
            </div>
            <div className="steps">
              {STEPS.map((label, i) => (
                <Step key={label} label={label} status={statusFor(i)} />
              ))}
            </div>
          </div>
        )}

        {jobs.length > 1 && (
          <div className="card">
            <h3>All jobs</h3>
            <div className="stack" style={{ gap: 6 }}>
              {jobs.slice(-8).reverse().map((j) => (
                <div key={j.id} className="row spread" style={{ fontSize: 13 }}>
                  <span style={{ color: "var(--text-secondary)" }}>
                    {j.status === "done" ? "✅" : j.status === "error" ? "❌" : "⏳"} {j.url}
                  </span>
                  <span className="step-dot" />
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </>
  );
}
