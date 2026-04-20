import { useMemo, useState } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const CK_GRADIENT = "linear-gradient(135deg, #FFB370 0%, #FF6B5A 40%, #F23D6D 100%)";

const COVER_GRADIENTS = [
  "linear-gradient(135deg, #FF9A76, #FF4F76)",
  "linear-gradient(135deg, #9E7AFF, #5E3BF5)",
  "linear-gradient(135deg, #FFD166, #F9A826)",
  "linear-gradient(135deg, #22D3A4, #0891B2)",
  "linear-gradient(135deg, #FB7185, #E11D48)",
  "linear-gradient(135deg, #A78BFA, #7C3AED)",
];

function coverGradient(seed) {
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  return COVER_GRADIENTS[h % COVER_GRADIENTS.length];
}

function formatDuration(sec) {
  const s = Math.max(0, Number(sec) || 0);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function FeaturedHero({ song, onPlay }) {
  const coverSrc = song.cover_path ? convertFileSrc(song.cover_path) : null;
  const bg = coverSrc
    ? `url(${coverSrc}) center/cover no-repeat`
    : coverGradient(song._dir || song.title || "x");

  return (
    <div style={{
      position: "relative", borderRadius: 24, overflow: "hidden",
      minHeight: 240, padding: 28, background: bg,
      display: "flex", alignItems: "flex-end",
      border: "1px solid rgba(255,255,255,0.06)",
    }}>
      <div style={{ position: "absolute", inset: 0, background: "linear-gradient(105deg, rgba(7,6,12,0.88) 0%, rgba(7,6,12,0.5) 50%, rgba(7,6,12,0.15) 100%)" }}/>
      <div style={{ position: "absolute", inset: 0, background: "radial-gradient(ellipse at 80% 20%, rgba(255,107,90,0.22), transparent 60%)" }}/>

      <div style={{ position: "relative", zIndex: 1, maxWidth: 520 }}>
        <div style={{
          display: "inline-flex", alignItems: "center", gap: 8,
          padding: "5px 10px", borderRadius: 999,
          background: "rgba(255,255,255,0.1)", backdropFilter: "blur(12px)",
          fontSize: 10.5, fontWeight: 700, letterSpacing: 1.4, color: "#FFF", textTransform: "uppercase",
          marginBottom: 16,
        }}>
          <span style={{ width: 6, height: 6, borderRadius: "50%", background: "#F23D6D", boxShadow: "0 0 8px #F23D6D" }}/>
          Tonight&apos;s pick
        </div>
        <h2 style={{
          margin: 0, fontFamily: "var(--font-display)", fontWeight: 700, fontSize: 48,
          lineHeight: 0.95, letterSpacing: -2, color: "#FFF",
          overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 500,
        }}>{song.title || "Unknown"}</h2>
        <div style={{ marginTop: 10, fontSize: 14, fontWeight: 500, color: "rgba(255,255,255,0.8)" }}>
          {song.artist || "Unknown"}{song.album ? <span style={{ color: "rgba(255,255,255,0.5)" }}> · {song.album}</span> : null}
        </div>
        <div style={{ display: "flex", gap: 10, marginTop: 22 }}>
          <button onClick={onPlay} style={{
            all: "unset", cursor: "pointer",
            display: "inline-flex", alignItems: "center", gap: 8,
            padding: "10px 18px", borderRadius: 12, fontSize: 13, fontWeight: 600,
            background: CK_GRADIENT, color: "#FFF",
            boxShadow: "0 8px 20px rgba(242,61,109,0.35), inset 0 1px 0 rgba(255,255,255,0.2)",
          }}>
            <svg width="14" height="14" viewBox="0 0 24 24"><path d="M7 4.5v15L20 12 7 4.5z" fill="currentColor"/></svg>
            Sing now
          </button>
        </div>
      </div>
    </div>
  );
}

function TrackCard({ song, onPlay, onDelete, onReprocess }) {
  const [hovered, setHovered] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [reprocessing, setReprocessing] = useState(false);
  const [reprocessMsg, setReprocessMsg] = useState("");
  const coverSrc = song.cover_path ? convertFileSrc(song.cover_path) : null;
  const bg = coverSrc ? `url(${coverSrc}) center/cover no-repeat` : coverGradient(song._dir || "x");

  const handleDelete = async (e) => {
    e.stopPropagation();
    if (!confirming) { setConfirming(true); return; }
    try {
      await invoke("delete_song", { dir: song._dir });
      onDelete(song._dir);
    } catch (err) {
      console.error("delete_song failed", err);
    }
  };

  const handleReprocess = async (e) => {
    e.stopPropagation();
    if (reprocessing) return;
    setReprocessing(true);
    setReprocessMsg("Starting…");
    let unlisten;
    try {
      unlisten = await listen("karaoke://reprocess-progress", (ev) => {
        const { message, status } = ev.payload || {};
        if (status === "error") setReprocessMsg("Error: " + message);
        else setReprocessMsg(message || "");
      });
      await invoke("reprocess_song", { dir: song._dir });
      setReprocessMsg("Done!");
      await onReprocess?.(song._dir);
    } catch (err) {
      setReprocessMsg("Error: " + err);
    } finally {
      unlisten?.();
      setTimeout(() => { setReprocessing(false); setReprocessMsg(""); }, 1500);
    }
  };

  return (
    <div
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => { setHovered(false); setConfirming(false); }}
      onClick={() => onPlay(song)}
      style={{
        borderRadius: 16, overflow: "hidden",
        background: "rgba(255,255,255,0.03)",
        border: "1px solid rgba(255,255,255,0.06)",
        transition: "all 180ms ease", cursor: "pointer",
        transform: hovered ? "translateY(-4px)" : "translateY(0)",
        boxShadow: hovered ? "0 16px 32px rgba(0,0,0,0.4)" : "none",
      }}
    >
      {/* Cover */}
      <div style={{ position: "relative", aspectRatio: "1", background: bg }}>
        <div style={{
          position: "absolute", inset: 0,
          background: "linear-gradient(180deg, transparent 40%, rgba(0,0,0,0.6) 100%)",
          opacity: hovered ? 1 : 0.7, transition: "opacity 180ms",
        }}/>
        {/* Play button */}
        <div style={{
          position: "absolute", bottom: 10, right: 10,
          width: 40, height: 40, borderRadius: "50%",
          background: CK_GRADIENT,
          display: "flex", alignItems: "center", justifyContent: "center",
          boxShadow: "0 8px 20px rgba(242,61,109,0.45)",
          transform: hovered ? "scale(1.15)" : "scale(1)",
          transition: "transform 180ms ease",
        }}>
          <svg width="14" height="14" viewBox="0 0 24 24"><path d="M7 4.5v15L20 12 7 4.5z" fill="#FFF"/></svg>
        </div>
        {/* Badges */}
        <div style={{ position: "absolute", top: 10, left: 10, display: "flex", gap: 5 }}>
          {song.lrc && (
            <span style={{
              display: "inline-flex", padding: "3px 8px", borderRadius: 999,
              fontSize: 9.5, fontWeight: 700, letterSpacing: 0.8, textTransform: "uppercase",
              background: CK_GRADIENT, color: "#FFF",
              boxShadow: "0 4px 10px rgba(242,61,109,0.3)",
            }}>LRC</span>
          )}
        </div>
      </div>
      {/* Info */}
      <div style={{ padding: 14 }}>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
          <div style={{ fontSize: 14, fontWeight: 700, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1 }}>
            {song.title || "Unknown"}
          </div>
          <span style={{ fontSize: 11, color: "rgba(237,233,255,0.45)", fontFamily: "var(--font-mono)", flexShrink: 0 }}>
            {formatDuration(song.duration_sec)}
          </span>
        </div>
        <div style={{ marginTop: 4, fontSize: 12, color: "rgba(237,233,255,0.55)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {song.artist || "Unknown"}
        </div>
        {/* Actions */}
        <div style={{ display: "flex", gap: 6, marginTop: 10, alignItems: "center" }}>
          <button
            onClick={handleDelete}
            onBlur={() => setConfirming(false)}
            style={{
              all: "unset", cursor: "pointer",
              fontSize: 11, padding: "4px 10px", borderRadius: 6,
              background: confirming ? "rgba(242,61,109,0.2)" : "rgba(255,255,255,0.04)",
              color: confirming ? "#F23D6D" : "rgba(237,233,255,0.45)",
              border: confirming ? "1px solid rgba(242,61,109,0.3)" : "1px solid rgba(255,255,255,0.06)",
              transition: "all 140ms",
            }}
          >
            {confirming ? "Confirm?" : "Delete"}
          </button>
          {reprocessing ? (
            <span style={{ fontSize: 10.5, color: "rgba(237,233,255,0.55)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1 }}>
              {reprocessMsg}
            </span>
          ) : (
            <button
              onClick={handleReprocess}
              title="Re-process lyrics & alignment"
              style={{
                all: "unset", cursor: "pointer",
                fontSize: 11, padding: "4px 10px", borderRadius: 6,
                background: "rgba(255,255,255,0.04)",
                color: "rgba(237,233,255,0.45)",
                border: "1px solid rgba(255,255,255,0.06)",
              }}
            >
              ↻ Re-process
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function AddTrackCard({ onAdd }) {
  const [hovered, setHovered] = useState(false);
  return (
    <div
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onClick={onAdd}
      style={{
        borderRadius: 16, aspectRatio: "1 / 1.25",
        border: "1.5px dashed rgba(255,107,90,0.3)",
        display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center",
        gap: 10, cursor: "pointer",
        background: hovered ? "rgba(255,107,90,0.08)" : "rgba(255,107,90,0.03)",
        borderColor: hovered ? "rgba(255,107,90,0.5)" : "rgba(255,107,90,0.3)",
        transition: "all 180ms",
      }}
    >
      <div style={{
        width: 48, height: 48, borderRadius: "50%",
        background: "rgba(255,107,90,0.12)",
        display: "flex", alignItems: "center", justifyContent: "center",
        color: "#FF9070", fontSize: 24,
      }}>+</div>
      <div style={{ fontSize: 13, fontWeight: 600, color: "#FF9070" }}>Add from YouTube</div>
      <div style={{ fontSize: 11, color: "rgba(237,233,255,0.4)", textAlign: "center", padding: "0 14px" }}>
        Paste a link, we&apos;ll do the rest
      </div>
    </div>
  );
}

const FILTERS = [
  { id: "all",    label: "All" },
  { id: "lrc",    label: "Synced lyrics" },
  { id: "recent", label: "Recently added" },
];

export default function Library({ songs, onPlay, onDelete, onRefresh, onAddSong, onReprocess }) {
  const [q, setQ] = useState("");
  const [filter, setFilter] = useState("all");

  const filtered = useMemo(() => {
    let list = songs;
    const needle = q.trim().toLowerCase();
    if (needle) list = list.filter(s => (s.title || "").toLowerCase().includes(needle) || (s.artist || "").toLowerCase().includes(needle));
    if (filter === "lrc") list = list.filter(s => Boolean(s.lrc));
    return list;
  }, [songs, q, filter]);

  const featured = songs[0] || null;

  return (
    <div style={{ padding: "28px 36px 36px", overflowY: "auto", height: "100%", boxSizing: "border-box" }}>
      {/* Header */}
      <div style={{ display: "flex", alignItems: "flex-end", gap: 16, marginBottom: 24 }}>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 2, color: "#FF9070", textTransform: "uppercase", marginBottom: 8 }}>
            Your library
          </div>
          <h1 style={{ margin: 0, fontFamily: "var(--font-display)", fontWeight: 700, fontSize: 36, letterSpacing: -1.2, lineHeight: 1, color: "#FFF" }}>
            Featured tonight
          </h1>
          <div style={{ marginTop: 8, fontSize: 13.5, color: "rgba(237,233,255,0.55)", fontWeight: 500 }}>
            {songs.length} tracks · {songs.filter(s => s.lrc).length} with word-sync lyrics
          </div>
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <GhostBtn onClick={onRefresh} icon={<RefreshIcon/>}>Refresh</GhostBtn>
          <PrimaryBtn onClick={onAddSong} icon={<PlusIcon/>}>Add song</PrimaryBtn>
        </div>
      </div>

      {/* Featured hero */}
      {featured && <FeaturedHero song={featured} onPlay={() => onPlay(featured)}/>}

      {/* Search + filters */}
      <div style={{ display: "flex", gap: 12, alignItems: "center", margin: "28px 0 18px" }}>
        <div style={{
          flex: 1, display: "flex", alignItems: "center", gap: 10,
          padding: "12px 16px", borderRadius: 12,
          background: "rgba(255,255,255,0.03)",
          border: "1px solid rgba(255,255,255,0.06)",
        }}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
            <circle cx="11" cy="11" r="7" stroke="rgba(237,233,255,0.5)" strokeWidth="1.8"/>
            <path d="M16 16l5 5" stroke="rgba(237,233,255,0.5)" strokeWidth="1.8" strokeLinecap="round"/>
          </svg>
          <input
            value={q} onChange={e => setQ(e.target.value)}
            placeholder="Search songs, artists…"
            style={{
              flex: 1, border: "none", outline: "none", background: "transparent",
              color: "#FFF", fontSize: 13.5, fontFamily: "var(--font-sans)", fontWeight: 500,
            }}
          />
          <kbd style={{
            fontSize: 10, padding: "3px 7px", borderRadius: 5,
            background: "rgba(255,255,255,0.06)", color: "rgba(237,233,255,0.5)",
            fontFamily: "var(--font-mono)", letterSpacing: 0.5,
          }}>⌘ K</kbd>
        </div>
        <div style={{ display: "flex", gap: 4, padding: 4, borderRadius: 10, background: "rgba(255,255,255,0.03)", border: "1px solid rgba(255,255,255,0.06)" }}>
          {FILTERS.map(f => (
            <button key={f.id} onClick={() => setFilter(f.id)} style={{
              all: "unset", cursor: "pointer",
              padding: "7px 14px", borderRadius: 7,
              fontSize: 12, fontWeight: 600,
              color: filter === f.id ? "#FFF" : "rgba(237,233,255,0.55)",
              background: filter === f.id ? "rgba(255,255,255,0.08)" : "transparent",
              transition: "all 140ms",
            }}>{f.label}</button>
          ))}
        </div>
      </div>

      {/* Grid */}
      {filtered.length === 0 ? (
        <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", padding: 48, color: "rgba(237,233,255,0.4)", textAlign: "center", gap: 12 }}>
          <div style={{ fontSize: 48, opacity: 0.5 }}>🎶</div>
          <div style={{ fontSize: 14 }}>No songs yet. Head to Download to add some.</div>
        </div>
      ) : (
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))", gap: 18 }}>
          {filtered.map(s => (
            <TrackCard key={s._dir} song={s} onPlay={onPlay} onDelete={onDelete} onReprocess={onReprocess}/>
          ))}
          <AddTrackCard onAdd={onAddSong}/>
        </div>
      )}
    </div>
  );
}

function GhostBtn({ children, onClick, icon }) {
  return (
    <button onClick={onClick} style={{
      all: "unset", cursor: "pointer",
      display: "inline-flex", alignItems: "center", gap: 8,
      padding: "10px 16px", borderRadius: 12,
      fontSize: 13, fontWeight: 600,
      background: "rgba(255,255,255,0.04)", color: "#EDE9FF",
      border: "1px solid rgba(255,255,255,0.08)",
    }}>{icon}{children}</button>
  );
}

function PrimaryBtn({ children, onClick, icon }) {
  return (
    <button onClick={onClick} style={{
      all: "unset", cursor: "pointer",
      display: "inline-flex", alignItems: "center", gap: 8,
      padding: "10px 16px", borderRadius: 12,
      fontSize: 13, fontWeight: 600,
      background: CK_GRADIENT, color: "#FFF",
      boxShadow: "0 8px 20px rgba(242,61,109,0.35), inset 0 1px 0 rgba(255,255,255,0.2)",
    }}>{icon}{children}</button>
  );
}

function RefreshIcon() {
  return <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
    <path d="M21 12a9 9 0 11-3.5-7.1M21 4v5h-5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"/>
  </svg>;
}
function PlusIcon() {
  return <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
    <path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round"/>
  </svg>;
}
