import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

function formatDate(ts) {
  if (!ts) return "";
  return new Date(ts * 1000).toLocaleDateString();
}

export default function Leaderboard({ songs }) {
  const [selected, setSelected] = useState(null);
  const [songTop, setSongTop] = useState([]);
  const [globalTop, setGlobalTop] = useState([]);

  useEffect(() => {
    (async () => {
      try {
        const g = await invoke("leaderboard_global_top", { limit: 20 });
        setGlobalTop(Array.isArray(g) ? g : []);
      } catch (e) { console.error(e); }
    })();
  }, []);

  useEffect(() => {
    if (!selected) { setSongTop([]); return; }
    (async () => {
      try {
        const t = await invoke("leaderboard_top", { songDir: selected._dir, limit: 10 });
        setSongTop(Array.isArray(t) ? t : []);
      } catch (e) { console.error(e); }
    })();
  }, [selected]);

  const songList = useMemo(
    () => (songs || []).slice().sort((a, b) => (a.title || "").localeCompare(b.title || "")),
    [songs]
  );

  return (
    <>
      <div className="main-header">
        <div>
          <h2>Classifica</h2>
          <p>Top scores per song + global leaderboard.</p>
        </div>
      </div>

      <div className="content" style={{ display: "grid", gridTemplateColumns: "260px 1fr", gap: 16 }}>
        <div className="card" style={{ maxHeight: "70vh", overflowY: "auto" }}>
          <h3>Songs</h3>
          <div className="stack" style={{ gap: 4 }}>
            <button
              className={`nav-btn ${!selected ? "active" : ""}`}
              onClick={() => setSelected(null)}
            >
              🌍 Global Top
            </button>
            {songList.map((s) => (
              <button
                key={s._dir}
                className={`nav-btn ${selected?._dir === s._dir ? "active" : ""}`}
                onClick={() => setSelected(s)}
                style={{ textAlign: "left" }}
              >
                <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {s.title}
                </span>
              </button>
            ))}
          </div>
        </div>

        <div className="card">
          <h3>{selected ? selected.title : "Global Top"}</h3>
          <ScoreTable entries={selected ? songTop : globalTop} />
        </div>
      </div>
    </>
  );
}

function ScoreTable({ entries }) {
  if (!entries || entries.length === 0) {
    return <div className="empty"><div className="icon">🏆</div><div>No scores yet.</div></div>;
  }
  return (
    <div className="stack" style={{ gap: 4 }}>
      <div className="row spread" style={{ fontSize: 12, color: "var(--text-muted)", padding: "6px 10px" }}>
        <span>#</span>
        <span style={{ flex: 1, paddingLeft: 12 }}>Player</span>
        <span style={{ flex: 1 }}>Song</span>
        <span>Hit/Par/Miss</span>
        <span>Date</span>
        <span>Score</span>
      </div>
      {entries.map((e, i) => (
        <div key={e.id || i} className="row spread" style={{ padding: "8px 10px", borderRadius: 8, background: "var(--bg-input)" }}>
          <span style={{ minWidth: 24, fontWeight: 700 }}>{i + 1}</span>
          <span style={{ flex: 1, paddingLeft: 12 }}>{e.player_name}</span>
          <span style={{ flex: 1, color: "var(--text-secondary)", fontSize: 13 }}>{e.song_title}</span>
          <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
            {e.hits}/{e.partials}/{e.misses}
          </span>
          <span style={{ fontSize: 12, color: "var(--text-muted)" }}>{formatDate(e.created_at)}</span>
          <span style={{ fontWeight: 700, color: "var(--secondary-2)" }}>{e.score}</span>
        </div>
      ))}
    </div>
  );
}
