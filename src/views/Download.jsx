import { useState } from "react";
import { useJobs } from "../jobsContext.jsx";

const CK_GRADIENT = "linear-gradient(135deg, #FFB370 0%, #FF6B5A 40%, #F23D6D 100%)";

const STEPS = [
  "Download audio",
  "Fetch lyrics",
  "Fetch album art",
  "Separate vocals",
  "Align words",
  "Compute pitch",
  "Save",
];

// Shorter labels for pipeline display
const STAGE_LABELS = ["download", "lyrics", "art", "separate", "align", "pitch", "save"];

function PipelineRow({ job }) {
  const stageIdx = job.current_step ?? 0;
  return (
    <div style={{
      padding: 14, borderRadius: 14,
      background: "rgba(255,255,255,0.03)",
      border: "1px solid rgba(255,255,255,0.06)",
      display: "flex", alignItems: "center", gap: 14,
    }}>
      <div style={{
        width: 44, height: 44, borderRadius: 10, flexShrink: 0,
        background: "linear-gradient(135deg, #FF9A76, #FF4F76)",
        display: "flex", alignItems: "center", justifyContent: "center", overflow: "hidden",
      }}>
        <span style={{ fontSize: 22 }}>🎵</span>
      </div>

      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
          <span style={{ fontSize: 13.5, fontWeight: 600, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1 }}>
            {job.url?.split("v=")[1] ? `youtube:${job.url.split("v=")[1].slice(0, 11)}` : (job.url || "Job")}
          </span>
          <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "#FF9070", fontWeight: 600, flexShrink: 0 }}>
            {Math.floor((job.progress || 0) * 100)}%
          </span>
        </div>

        {/* Pipeline stages */}
        <div style={{ display: "flex", alignItems: "center", gap: 4, flexWrap: "nowrap", overflow: "hidden" }}>
          {STAGE_LABELS.map((s, i) => {
            const done = job.status === "done" || i < stageIdx;
            const active = !done && i === stageIdx && job.status === "active";
            return (
              <div key={s} style={{ display: "flex", alignItems: "center", gap: 4, flexShrink: i < 3 ? 1 : 0, minWidth: 0 }}>
                <div style={{
                  display: "flex", alignItems: "center", gap: 4,
                  fontSize: 10, fontWeight: 600, textTransform: "capitalize", letterSpacing: 0.3,
                  color: done ? "#22D3A4" : active ? "#FF9070" : "rgba(237,233,255,0.35)",
                  whiteSpace: "nowrap",
                }}>
                  <span style={{
                    width: 6, height: 6, borderRadius: "50%", flexShrink: 0,
                    background: done ? "#22D3A4" : active ? "#FF9070" : "rgba(255,255,255,0.2)",
                    boxShadow: active ? "0 0 8px #FF9070" : "none",
                  }}/>
                  {s}
                </div>
                {i < STAGE_LABELS.length - 1 && (
                  <div style={{ width: 16, height: 2, borderRadius: 1, background: done ? "#22D3A4" : "rgba(255,255,255,0.08)", flexShrink: 0 }}/>
                )}
              </div>
            );
          })}
        </div>

        {job.message && (
          <div style={{ marginTop: 6, fontSize: 11, color: job.status === "error" ? "#F23D6D" : "rgba(237,233,255,0.5)" }}>
            {job.message}
          </div>
        )}

        {/* Progress bar */}
        <div style={{ marginTop: 8, height: 3, borderRadius: 2, background: "rgba(255,255,255,0.06)", overflow: "hidden" }}>
          <div style={{ width: `${Math.min(100, (job.progress || 0) * 100)}%`, height: "100%", background: job.status === "error" ? "#F23D6D" : CK_GRADIENT, transition: "width 300ms ease" }}/>
        </div>
      </div>
    </div>
  );
}

export default function Download() {
  const { jobs, enqueue, cancel } = useJobs();
  const [url, setUrl] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [lastJobId, setLastJobId] = useState(null);

  const activeJobs = jobs.filter(j => j.status === "active" || j.status === "queued");
  const doneJobs = jobs.filter(j => j.status === "done" || j.status === "error");
  const currentJob = jobs.find(j => j.id === lastJobId);

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

  return (
    <div style={{ padding: "28px 36px 36px", overflowY: "auto", height: "100%", boxSizing: "border-box" }}>
      {/* Header */}
      <div style={{ marginBottom: 28 }}>
        <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 2, color: "#FF9070", textTransform: "uppercase", marginBottom: 8 }}>
          Import
        </div>
        <h1 style={{ margin: 0, fontFamily: "var(--font-display)", fontWeight: 700, fontSize: 36, letterSpacing: -1.2, lineHeight: 1, color: "#FFF" }}>
          Download queue
        </h1>
        <div style={{ marginTop: 8, fontSize: 13.5, color: "rgba(237,233,255,0.55)", fontWeight: 500 }}>
          Paste a YouTube link — we strip the instrumental and time the lyrics for you.
        </div>
      </div>

      {/* Input card */}
      <div style={{
        padding: 20, borderRadius: 20,
        background: "linear-gradient(105deg, rgba(255,107,90,0.1), rgba(255,255,255,0.02))",
        border: "1px solid rgba(255,107,90,0.2)",
        display: "flex", flexDirection: "column", gap: 14,
      }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
            <rect x="2" y="5" width="20" height="14" rx="4" fill="#F23D6D"/>
            <path d="M10 9v6l5-3-5-3z" fill="#FFF"/>
          </svg>
          <span style={{ fontSize: 13, fontWeight: 600, color: "#EDE9FF" }}>YouTube URL</span>
        </div>
        <div style={{ display: "flex", gap: 10 }}>
          <input
            value={url} onChange={e => setUrl(e.target.value)}
            onKeyDown={e => e.key === "Enter" && start()}
            placeholder="https://www.youtube.com/watch?v=…"
            disabled={submitting}
            style={{
              flex: 1, padding: "13px 16px", borderRadius: 12,
              background: "rgba(7,6,12,0.7)",
              border: "1px solid rgba(255,255,255,0.08)",
              color: "#FFF", fontSize: 13.5, fontFamily: "var(--font-mono)", outline: "none",
            }}
          />
          <button onClick={start} disabled={submitting || !url.trim()} style={{
            all: "unset", cursor: (submitting || !url.trim()) ? "default" : "pointer",
            display: "inline-flex", alignItems: "center", gap: 8,
            padding: "0 20px", borderRadius: 12, flexShrink: 0,
            fontSize: 13, fontWeight: 600, color: "#FFF",
            background: CK_GRADIENT,
            boxShadow: "0 8px 20px rgba(242,61,109,0.35), inset 0 1px 0 rgba(255,255,255,0.2)",
            opacity: (submitting || !url.trim()) ? 0.55 : 1,
          }}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
              <path d="M5 19l3-3m8-13s4 1 5 5l-6 6-4-4 5-7zM14 15l2 2-3 3-1-2m-5-5l-2-2 3-3 2 1" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"/>
            </svg>
            {submitting ? "Queuing…" : "Queue download"}
          </button>
        </div>
      </div>

      {/* Active jobs */}
      {activeJobs.length > 0 && (
        <div style={{ marginTop: 32 }}>
          <div style={{ display: "flex", alignItems: "baseline", gap: 10, marginBottom: 14 }}>
            <h3 style={{ margin: 0, fontFamily: "var(--font-display)", fontSize: 18, color: "#FFF", fontWeight: 700 }}>
              In progress
            </h3>
            <span style={{ fontSize: 11, color: "rgba(237,233,255,0.45)", fontWeight: 600 }}>
              {activeJobs.length} active · runs in background
            </span>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            {activeJobs.map(j => (
              <div key={j.id} style={{ position: "relative" }}>
                <PipelineRow job={j}/>
                {(j.status === "active" || j.status === "queued") && (
                  <button onClick={() => cancel(j.id)} style={{
                    all: "unset", cursor: "pointer",
                    position: "absolute", top: 14, right: 14,
                    width: 24, height: 24, borderRadius: 6,
                    display: "flex", alignItems: "center", justifyContent: "center",
                    background: "rgba(255,255,255,0.05)", color: "rgba(237,233,255,0.5)",
                    fontSize: 14,
                  }}>×</button>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Current tracked job if not in active (queued/error state) */}
      {currentJob && !activeJobs.find(j => j.id === currentJob.id) && currentJob.status !== "done" && (
        <div style={{ marginTop: 32 }}>
          <h3 style={{ margin: "0 0 14px", fontFamily: "var(--font-display)", fontSize: 18, color: "#FFF", fontWeight: 700 }}>Current job</h3>
          <PipelineRow job={currentJob}/>
        </div>
      )}

      {/* History */}
      {doneJobs.length > 0 && (
        <div style={{ marginTop: 32 }}>
          <h3 style={{ margin: "0 0 14px", fontFamily: "var(--font-display)", fontSize: 18, color: "#FFF", fontWeight: 700 }}>
            Recently imported
          </h3>
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            {doneJobs.slice(-8).reverse().map(j => (
              <div key={j.id} style={{
                display: "flex", alignItems: "center", gap: 12,
                padding: "10px 14px", borderRadius: 12,
                background: "rgba(255,255,255,0.02)",
                border: "1px solid rgba(255,255,255,0.04)",
              }}>
                <div style={{
                  width: 28, height: 28, borderRadius: 8, flexShrink: 0,
                  background: j.status === "done" ? "rgba(34,211,164,0.15)" : "rgba(242,61,109,0.15)",
                  color: j.status === "done" ? "#22D3A4" : "#F23D6D",
                  display: "flex", alignItems: "center", justifyContent: "center",
                  fontSize: 12,
                }}>
                  {j.status === "done" ? "✓" : "✕"}
                </div>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 12.5, fontWeight: 600, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {j.url}
                  </div>
                  {j.message && (
                    <div style={{ fontSize: 11.5, color: j.status === "error" ? "#F23D6D" : "rgba(237,233,255,0.5)" }}>
                      {j.message}
                    </div>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
