import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

const CK_GRADIENT = "linear-gradient(135deg, #FFB370 0%, #FF6B5A 40%, #F23D6D 100%)";

function formatDate(ts) {
  if (!ts) return "";
  return new Date(ts * 1000).toLocaleDateString("it-IT");
}

function Avatar({ name }) {
  const hue = [...(name || "?")].reduce((a, c) => a + c.charCodeAt(0), 0) * 37 % 360;
  return (
    <div style={{
      width: 24, height: 24, borderRadius: "50%", flexShrink: 0,
      background: `hsl(${hue}, 70%, 55%)`,
      fontSize: 10, fontWeight: 700, color: "#FFF",
      display: "flex", alignItems: "center", justifyContent: "center",
    }}>{(name || "?")[0]}</div>
  );
}

function Podium({ entries }) {
  if (!entries || entries.length === 0) return null;
  const order = [entries[1], entries[0], entries[2]].filter(Boolean);
  const heights = [120, 150, 100];
  const medals = ["🥈", "🥇", "🥉"];
  const tints = ["#C9D2E2", "#FFD166", "#E29E7A"];
  const displayOrder = entries[1] ? [entries[1], entries[0], entries[2]] : [null, entries[0], entries[2]];

  return (
    <div style={{
      display: "flex", alignItems: "flex-end", justifyContent: "center", gap: 16,
      padding: 24, borderRadius: 20,
      background: "radial-gradient(ellipse at 50% 0%, rgba(255,107,90,0.12), transparent 60%), linear-gradient(180deg, rgba(255,255,255,0.02), transparent)",
      border: "1px solid rgba(255,255,255,0.06)",
      marginBottom: 28,
    }}>
      {[displayOrder[0], displayOrder[1], displayOrder[2]].map((e, i) => {
        if (!e) return <div key={i} style={{ width: 180 }}/>;
        const isFirst = i === 1;
        return (
          <div key={i} style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 10, width: 180 }}>
            <div style={{
              width: isFirst ? 56 : 44, height: isFirst ? 56 : 44, borderRadius: "50%",
              background: `hsl(${i * 120}, 70%, 55%)`,
              display: "flex", alignItems: "center", justifyContent: "center",
              fontSize: isFirst ? 20 : 16, fontWeight: 700, color: "#FFF",
              border: `3px solid ${tints[i]}`,
              boxShadow: `0 8px 20px hsl(${i * 120}, 70%, 55%, 0.35)`,
            }}>{e.player_name?.[0] || "?"}</div>
            <div style={{ textAlign: "center" }}>
              <div style={{ fontSize: 13, fontWeight: 700, color: "#FFF" }}>{e.player_name}</div>
              <div style={{ fontSize: 10.5, color: "rgba(237,233,255,0.5)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 160 }}>{e.song_title}</div>
            </div>
            <div style={{
              width: "100%", height: heights[i], borderRadius: "12px 12px 0 0",
              background: isFirst
                ? "linear-gradient(180deg, rgba(255,209,102,0.3), rgba(255,107,90,0.12))"
                : "linear-gradient(180deg, rgba(255,255,255,0.07), rgba(255,255,255,0.02))",
              border: `1px solid ${isFirst ? "rgba(255,209,102,0.3)" : "rgba(255,255,255,0.07)"}`,
              borderBottom: "none",
              display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 4,
            }}>
              <div style={{ fontSize: 24 }}>{medals[i]}</div>
              <div style={{ fontFamily: "var(--font-display)", fontSize: isFirst ? 24 : 18, fontWeight: 700, color: "#FFF", letterSpacing: -0.8 }}>
                {e.score}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

function LeaderboardRow({ entry, idx }) {
  const [hov, setHov] = useState(false);
  return (
    <div
      onMouseEnter={() => setHov(true)} onMouseLeave={() => setHov(false)}
      style={{
        display: "grid", gridTemplateColumns: "40px 1.4fr 1.6fr 1fr 1fr 0.8fr",
        padding: "11px 18px", alignItems: "center",
        borderBottom: "1px solid rgba(255,255,255,0.03)",
        background: hov ? "rgba(255,255,255,0.02)" : "transparent",
        fontSize: 13, transition: "background 140ms",
      }}
    >
      <span style={{ fontFamily: "var(--font-mono)", fontSize: 12, color: idx < 3 ? "#FFD166" : "rgba(237,233,255,0.5)", fontWeight: 700 }}>
        {String(idx + 1).padStart(2, "0")}
      </span>
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <Avatar name={entry.player_name}/>
        <span style={{ fontWeight: 600, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{entry.player_name}</span>
      </div>
      <span style={{ color: "rgba(237,233,255,0.7)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{entry.song_title}</span>
      <div style={{ display: "flex", gap: 5, alignItems: "center" }}>
        <span style={{ color: "#22D3A4", fontFamily: "var(--font-mono)", fontSize: 12 }}>{entry.hits}</span>
        <span style={{ color: "rgba(237,233,255,0.3)" }}>·</span>
        <span style={{ color: "#FFB370", fontFamily: "var(--font-mono)", fontSize: 12 }}>{entry.partials}</span>
        <span style={{ color: "rgba(237,233,255,0.3)" }}>·</span>
        <span style={{ color: "#F23D6D", fontFamily: "var(--font-mono)", fontSize: 12 }}>{entry.misses}</span>
      </div>
      <span style={{ color: "rgba(237,233,255,0.55)", fontFamily: "var(--font-mono)", fontSize: 11.5 }}>{formatDate(entry.created_at)}</span>
      <div style={{ textAlign: "right" }}>
        <span style={{ fontFamily: "var(--font-display)", fontWeight: 700, fontSize: 15, color: "#FFF", letterSpacing: -0.3 }}>
          {entry.score}
        </span>
      </div>
    </div>
  );
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

  const entries = selected ? songTop : globalTop;
  const top3 = entries.slice(0, 3);

  return (
    <div style={{ padding: "28px 36px 36px", overflowY: "auto", height: "100%", boxSizing: "border-box" }}>
      {/* Header */}
      <div style={{ display: "flex", alignItems: "flex-end", gap: 16, marginBottom: 24 }}>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 2, color: "#FF9070", textTransform: "uppercase", marginBottom: 8 }}>Leaderboard</div>
          <h1 style={{ margin: 0, fontFamily: "var(--font-display)", fontWeight: 700, fontSize: 36, letterSpacing: -1.2, lineHeight: 1, color: "#FFF" }}>
            Classifica
          </h1>
          <div style={{ marginTop: 8, fontSize: 13.5, color: "rgba(237,233,255,0.55)", fontWeight: 500 }}>
            Top scores per song + global leaderboard.
          </div>
        </div>
      </div>

      {/* Podium */}
      {top3.length > 0 && <Podium entries={top3}/>}

      {/* Body */}
      <div style={{ display: "grid", gridTemplateColumns: "220px 1fr", gap: 18 }}>
        {/* Song list sidebar */}
        <div style={{
          padding: 10, borderRadius: 16,
          background: "rgba(255,255,255,0.03)",
          border: "1px solid rgba(255,255,255,0.06)",
          display: "flex", flexDirection: "column", gap: 4,
          height: "fit-content",
        }}>
          <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 1.4, color: "rgba(237,233,255,0.5)", textTransform: "uppercase", padding: "8px 10px" }}>
            Songs
          </div>
          <button onClick={() => setSelected(null)} style={{
            all: "unset", cursor: "pointer",
            padding: "10px 12px", borderRadius: 10,
            display: "flex", alignItems: "center", gap: 8,
            background: !selected ? CK_GRADIENT : "transparent",
            color: !selected ? "#FFF" : "rgba(237,233,255,0.75)",
            boxShadow: !selected ? "0 6px 16px rgba(242,61,109,0.3)" : "none",
            fontSize: 13, fontWeight: 600, transition: "all 140ms",
          }}>
            <span>🌍</span> Global Top
          </button>
          {songList.map(s => {
            const active = selected?._dir === s._dir;
            return (
              <button key={s._dir} onClick={() => setSelected(s)} style={{
                all: "unset", cursor: "pointer",
                padding: "10px 12px", borderRadius: 10,
                display: "flex", flexDirection: "column", gap: 2,
                background: active ? CK_GRADIENT : "transparent",
                color: active ? "#FFF" : "rgba(237,233,255,0.75)",
                boxShadow: active ? "0 6px 16px rgba(242,61,109,0.3)" : "none",
                fontSize: 13, fontWeight: 600, transition: "all 140ms",
              }}>
                <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{s.title}</span>
                {s.artist && <span style={{ fontSize: 10.5, fontWeight: 500, color: active ? "rgba(255,255,255,0.7)" : "rgba(237,233,255,0.4)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{s.artist}</span>}
              </button>
            );
          })}
        </div>

        {/* Table */}
        <div style={{ borderRadius: 18, background: "rgba(255,255,255,0.03)", border: "1px solid rgba(255,255,255,0.06)", overflow: "hidden" }}>
          <div style={{ padding: 18, borderBottom: "1px solid rgba(255,255,255,0.05)", display: "flex", alignItems: "center", gap: 10 }}>
            <h3 style={{ margin: 0, fontFamily: "var(--font-display)", fontSize: 18, color: "#FFF", fontWeight: 700 }}>
              {selected ? selected.title : "Global Top"}
            </h3>
            <span style={{ fontSize: 11, padding: "3px 8px", borderRadius: 999, background: "rgba(34,211,164,0.12)", color: "#22D3A4", fontWeight: 700, letterSpacing: 0.4 }}>
              {entries.length} entries
            </span>
          </div>
          {/* Column headers */}
          <div style={{
            display: "grid", gridTemplateColumns: "40px 1.4fr 1.6fr 1fr 1fr 0.8fr",
            padding: "10px 18px",
            fontSize: 10, fontWeight: 700, letterSpacing: 1.4, textTransform: "uppercase",
            color: "rgba(237,233,255,0.4)",
            borderBottom: "1px solid rgba(255,255,255,0.05)",
          }}>
            <span>#</span>
            <span>Player</span>
            <span>Song</span>
            <span>Hit · Par · Miss</span>
            <span>Date</span>
            <span style={{ textAlign: "right" }}>Score</span>
          </div>
          {entries.length === 0 ? (
            <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", padding: 48, color: "rgba(237,233,255,0.4)", gap: 12 }}>
              <div style={{ fontSize: 40 }}>🏆</div>
              <div>No scores yet.</div>
            </div>
          ) : (
            entries.map((e, i) => <LeaderboardRow key={e.id || i} entry={e} idx={i}/>)
          )}
        </div>
      </div>
    </div>
  );
}
