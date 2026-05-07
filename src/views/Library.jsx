import { useCallback, useEffect, useMemo, useState } from "react";
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

function CloudIcon({ size = 12 }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path d="M18 10a6 6 0 00-11.47-2.44A5 5 0 107 20h11a4 4 0 000-8z"
        stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"/>
    </svg>
  );
}

function CloudBadge({ song }) {
  if (!song.cloud_synced && !song.local_deleted) return null;
  const isCloudOnly = song.local_deleted;
  return (
    <span style={{
      display: "inline-flex", alignItems: "center", gap: 3,
      padding: "3px 7px", borderRadius: 999,
      fontSize: 9.5, fontWeight: 700, letterSpacing: 0.8, textTransform: "uppercase",
      background: isCloudOnly ? "rgba(34,211,164,0.15)" : "rgba(96,200,255,0.12)",
      color: isCloudOnly ? "#22D3A4" : "#60C8FF",
      border: `1px solid ${isCloudOnly ? "rgba(34,211,164,0.3)" : "rgba(96,200,255,0.25)"}`,
      flexShrink: 0,
    }}>
      <CloudIcon size={9}/>
      {isCloudOnly ? "Cloud only" : "Synced"}
    </span>
  );
}

function LrcBadge({ song }) {
  if (!song.lrc) return null;
  return (
    <span style={{
      display: "inline-flex", padding: "3px 8px", borderRadius: 999,
      fontSize: 9.5, fontWeight: 700, letterSpacing: 0.8, textTransform: "uppercase",
      background: song.lrc_enhanced ? CK_GRADIENT : "rgba(0,0,0,0.55)",
      color: "#FFF",
      boxShadow: song.lrc_enhanced ? "0 4px 10px rgba(242,61,109,0.3)" : "0 2px 6px rgba(0,0,0,0.3)",
      border: song.lrc_enhanced ? "none" : "1px solid rgba(255,255,255,0.18)",
      flexShrink: 0,
    }}>
      {song.lrc_enhanced ? "LRC Enhanced" : "LRC"}
    </span>
  );
}

function TrackCard({ song, onPlay, onDelete, onReprocess }) {
  const [hovered, setHovered] = useState(false);
  // confirming: false | "local" | "everywhere"
  const [confirming, setConfirming] = useState(false);
  const [reprocessing, setReprocessing] = useState(false);
  const [reprocessMsg, setReprocessMsg] = useState("");
  const [cloudBusy, setCloudBusy] = useState(false);
  const [cloudMsg, setCloudMsg] = useState("");
  const coverSrc = song.cover_path ? convertFileSrc(song.cover_path) : null;
  const bg = coverSrc ? `url(${coverSrc}) center/cover no-repeat` : coverGradient(song._dir || "x");

  const handleDelete = async (e) => {
    e.stopPropagation();
    if (song.cloud_synced) {
      // Synced song: first confirm, then offer local-only vs everywhere
      if (!confirming) { setConfirming("choose"); return; }
      if (confirming === "choose") return; // wait for sub-choice
      if (confirming === "local") {
        // Keep on cloud, just remove local files
        try {
          await invoke("cloud_make_local_only", { dir: song._dir });
          await onReprocess?.(song._dir);
        } catch (err) { console.error(err); }
        setConfirming(false);
        return;
      }
      if (confirming === "everywhere") {
        try {
          await invoke("cloud_delete_song", { dir: song._dir });
          await invoke("delete_song", { dir: song._dir });
          onDelete(song._dir);
        } catch (err) { console.error(err); }
        setConfirming(false);
        return;
      }
    }
    // Not synced: normal delete
    if (!confirming) { setConfirming("confirm"); return; }
    try {
      await invoke("delete_song", { dir: song._dir });
      onDelete(song._dir);
    } catch (err) { console.error("delete_song failed", err); }
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

  const handleCloudSync = async (e) => {
    e.stopPropagation();
    if (cloudBusy) return;
    setCloudBusy(true); setCloudMsg("Sync...");
    try {
      await invoke("cloud_sync_song", { dir: song._dir });
      setCloudMsg("Synced!");
      await onReprocess?.(song._dir);
    } catch (err) { setCloudMsg("Errore: " + err); }
    finally { setTimeout(() => { setCloudBusy(false); setCloudMsg(""); }, 2000); }
  };

  const handleCloudDownload = async (e) => {
    e.stopPropagation();
    if (cloudBusy) return;
    setCloudBusy(true); setCloudMsg("Download...");
    try {
      await invoke("cloud_download_song", { dir: song._dir });
      setCloudMsg("Scaricato!");
      await onReprocess?.(song._dir);
    } catch (err) { setCloudMsg("Errore: " + err); }
    finally { setTimeout(() => { setCloudBusy(false); setCloudMsg(""); }, 2000); }
  };

  const handleMakeCloudOnly = async (e) => {
    e.stopPropagation();
    if (!song.cloud_synced) { setCloudMsg("Prima sincronizza!"); return; }
    try {
      await invoke("cloud_make_local_only", { dir: song._dir });
      await onReprocess?.(song._dir);
    } catch (err) { setCloudMsg("Errore: " + err); }
  };

  return (
    <div
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => { setHovered(false); setConfirming(false); }}
      onClick={() => { if (!song.local_deleted) onPlay(song); }}
      style={{
        borderRadius: 16, overflow: "hidden",
        background: "rgba(255,255,255,0.03)",
        border: song.local_deleted ? "1px solid rgba(34,211,164,0.2)" : "1px solid rgba(255,255,255,0.06)",
        transition: "all 180ms ease",
        cursor: song.local_deleted ? "default" : "pointer",
        transform: hovered && !song.local_deleted ? "translateY(-4px)" : "translateY(0)",
        boxShadow: hovered && !song.local_deleted ? "0 16px 32px rgba(0,0,0,0.4)" : "none",
      }}
    >
      <div style={{ position: "relative", aspectRatio: "1", background: bg }}>
        <div style={{
          position: "absolute", inset: 0,
          background: "linear-gradient(180deg, transparent 40%, rgba(0,0,0,0.6) 100%)",
          opacity: hovered ? 1 : 0.7, transition: "opacity 180ms",
        }}/>
        {song.local_deleted ? (
          <div style={{
            position: "absolute", bottom: 10, right: 10,
            padding: "6px 10px", borderRadius: 8,
            background: "rgba(34,211,164,0.18)",
            border: "1px solid rgba(34,211,164,0.3)",
            display: "flex", alignItems: "center", gap: 4,
            fontSize: 11, fontWeight: 700, color: "#22D3A4",
          }}>
            <CloudIcon size={11}/> Cloud only
          </div>
        ) : (
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
        )}
        <div style={{ position: "absolute", top: 10, left: 10, display: "flex", gap: 4, flexWrap: "wrap" }}>
          <LrcBadge song={song}/>
          <CloudBadge song={song}/>
        </div>
      </div>
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
        <div style={{ display: "flex", gap: 6, marginTop: 10, alignItems: "center", flexWrap: "wrap" }}>
          {!song.local_deleted && (
            confirming === "choose" ? (
              <div style={{ display: "flex", gap: 4 }}>
                <button onClick={(e) => { e.stopPropagation(); setConfirming("local"); handleDelete(e); }}
                  style={{ all: "unset", cursor: "pointer", fontSize: 10, padding: "4px 8px", borderRadius: 6, background: "rgba(255,183,112,0.15)", color: "#FFB370", border: "1px solid rgba(255,183,112,0.3)" }}>
                  Solo locale
                </button>
                <button onClick={(e) => { e.stopPropagation(); setConfirming("everywhere"); handleDelete(e); }}
                  style={{ all: "unset", cursor: "pointer", fontSize: 10, padding: "4px 8px", borderRadius: 6, background: "rgba(242,61,109,0.15)", color: "#F23D6D", border: "1px solid rgba(242,61,109,0.3)" }}>
                  Ovunque
                </button>
                <button onClick={(e) => { e.stopPropagation(); setConfirming(false); }}
                  style={{ all: "unset", cursor: "pointer", fontSize: 10, padding: "4px 8px", borderRadius: 6, background: "rgba(255,255,255,0.04)", color: "rgba(237,233,255,0.45)", border: "1px solid rgba(255,255,255,0.06)" }}>
                  ✕
                </button>
              </div>
            ) : (
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
                {confirming === "confirm" ? "Confirm?" : "Delete"}
              </button>
            )
          )}
          {cloudBusy ? (
            <span style={{ fontSize: 10.5, color: "rgba(237,233,255,0.55)", flex: 1 }}>{cloudMsg}</span>
          ) : song.local_deleted ? (
            <button onClick={handleCloudDownload} style={smallBtnStyle}>
              <CloudIcon size={10}/> Download
            </button>
          ) : song.cloud_synced ? (
            <button onClick={handleMakeCloudOnly} title="Rimuovi file locali" style={smallBtnStyle}>
              <CloudIcon size={10}/> Cloud only
            </button>
          ) : (
            <button onClick={handleCloudSync} style={smallBtnStyle}>
              <CloudIcon size={10}/> Sync
            </button>
          )}
          {cloudMsg && !cloudBusy && (
            <span style={{ fontSize: 10.5, color: "rgba(237,233,255,0.45)" }}>{cloudMsg}</span>
          )}
          {!song.local_deleted && !cloudBusy && (
            reprocessing ? (
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
            )
          )}
        </div>
      </div>
    </div>
  );
}

const smallBtnStyle = {
  all: "unset", cursor: "pointer",
  display: "inline-flex", alignItems: "center", gap: 4,
  fontSize: 11, padding: "4px 10px", borderRadius: 6,
  background: "rgba(255,255,255,0.04)",
  color: "rgba(237,233,255,0.45)",
  border: "1px solid rgba(255,255,255,0.06)",
};

function TrackRow({ song, onPlay, onDelete, onReprocess }) {
  const [hovered, setHovered] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [reprocessing, setReprocessing] = useState(false);
  const [reprocessMsg, setReprocessMsg] = useState("");
  const [cloudBusy, setCloudBusy] = useState(false);
  const [cloudMsg, setCloudMsg] = useState("");
  const coverSrc = song.cover_path ? convertFileSrc(song.cover_path) : null;
  const bg = coverSrc ? `url(${coverSrc}) center/cover no-repeat` : coverGradient(song._dir || "x");

  const handleDelete = async (e) => {
    e.stopPropagation();
    if (song.cloud_synced && !song.local_deleted) {
      if (!confirming) { setConfirming("choose"); return; }
      if (confirming === "choose") return;
      if (confirming === "local") {
        try {
          await invoke("cloud_make_local_only", { dir: song._dir });
          await onReprocess?.(song._dir);
        } catch (err) { console.error("make_cloud_only failed", err); }
        setConfirming(false);
        return;
      }
      if (confirming === "everywhere") {
        try {
          await invoke("cloud_delete_song", { dir: song._dir });
          await invoke("delete_song", { dir: song._dir });
          onDelete(song._dir);
        } catch (err) { console.error("delete everywhere failed", err); }
        setConfirming(false);
        return;
      }
    }
    if (!confirming) { setConfirming("confirm"); return; }
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

  const handleCloudSync = async (e) => {
    e.stopPropagation();
    if (cloudBusy) return;
    setCloudBusy(true); setCloudMsg("Sync...");
    try {
      await invoke("cloud_sync_song", { dir: song._dir });
      setCloudMsg("Synced!");
      await onReprocess?.(song._dir);
    } catch (err) { setCloudMsg("Errore: " + err); }
    finally { setTimeout(() => { setCloudBusy(false); setCloudMsg(""); }, 2000); }
  };

  const handleCloudDownload = async (e) => {
    e.stopPropagation();
    if (cloudBusy) return;
    setCloudBusy(true); setCloudMsg("Download...");
    try {
      await invoke("cloud_download_song", { dir: song._dir });
      setCloudMsg("Scaricato!");
      await onReprocess?.(song._dir);
    } catch (err) { setCloudMsg("Errore: " + err); }
    finally { setTimeout(() => { setCloudBusy(false); setCloudMsg(""); }, 2000); }
  };

  const handleMakeCloudOnly = async (e) => {
    e.stopPropagation();
    if (!song.cloud_synced) { setCloudMsg("Prima sincronizza!"); return; }
    try {
      await invoke("cloud_make_local_only", { dir: song._dir });
      await onReprocess?.(song._dir);
    } catch (err) { setCloudMsg("Errore: " + err); }
  };

  return (
    <div
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => { setHovered(false); setConfirming(false); }}
      onClick={() => { if (!song.local_deleted) onPlay(song); }}
      style={{
        display: "flex", alignItems: "center", gap: 12,
        padding: "8px 12px", borderRadius: 10,
        cursor: song.local_deleted ? "default" : "pointer",
        background: hovered ? "rgba(255,255,255,0.05)" : "transparent",
        transition: "background 140ms",
      }}
    >
      {/* Thumbnail */}
      <div style={{
        width: 44, height: 44, borderRadius: 8, flexShrink: 0,
        background: bg,
        position: "relative", overflow: "hidden",
      }}>
        {hovered && !song.local_deleted && (
          <div style={{
            position: "absolute", inset: 0,
            background: "rgba(0,0,0,0.45)",
            display: "flex", alignItems: "center", justifyContent: "center",
          }}>
            <svg width="12" height="12" viewBox="0 0 24 24"><path d="M7 4.5v15L20 12 7 4.5z" fill="#FFF"/></svg>
          </div>
        )}
        {song.local_deleted && (
          <div style={{
            position: "absolute", inset: 0,
            background: "rgba(34,211,164,0.12)",
            display: "flex", alignItems: "center", justifyContent: "center",
            color: "#22D3A4",
          }}>
            <CloudIcon size={16}/>
          </div>
        )}
      </div>

      {/* Title + artist */}
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 13.5, fontWeight: 600, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {song.title || "Unknown"}
        </div>
        <div style={{ fontSize: 11.5, color: "rgba(237,233,255,0.5)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", marginTop: 2 }}>
          {song.artist || "Unknown"}
        </div>
      </div>

      {/* Badges */}
      <LrcBadge song={song}/>
      <CloudBadge song={song}/>

      {/* Duration */}
      <span style={{ fontSize: 11, color: "rgba(237,233,255,0.4)", fontFamily: "var(--font-mono)", flexShrink: 0, width: 36, textAlign: "right" }}>
        {formatDuration(song.duration_sec)}
      </span>

      {/* Actions (visible on hover) */}
      <div style={{
        display: "flex", gap: 4, flexShrink: 0,
        opacity: hovered ? 1 : 0, transition: "opacity 140ms",
      }} onClick={e => e.stopPropagation()}>
        {cloudBusy ? (
          <span style={{ fontSize: 10, color: "rgba(237,233,255,0.5)", maxWidth: 80 }}>{cloudMsg}</span>
        ) : song.local_deleted ? (
          <button onClick={handleCloudDownload} style={{ ...smallBtnStyle, fontSize: 10.5, padding: "3px 8px", display: "inline-flex", gap: 3 }}>
            <CloudIcon size={9}/> ↓
          </button>
        ) : song.cloud_synced ? (
          <button onClick={handleMakeCloudOnly} title="Cloud only" style={{ all: "unset", cursor: "pointer", fontSize: 10.5, padding: "3px 8px", borderRadius: 5, background: "rgba(96,200,255,0.08)", color: "#60C8FF", border: "1px solid rgba(96,200,255,0.2)", display: "inline-flex", alignItems: "center", gap: 3 }}>
            <CloudIcon size={9}/>
          </button>
        ) : (
          <button onClick={handleCloudSync} title="Sync to MEGA" style={{ all: "unset", cursor: "pointer", fontSize: 10.5, padding: "3px 8px", borderRadius: 5, background: "rgba(255,255,255,0.04)", color: "rgba(237,233,255,0.45)", border: "1px solid rgba(255,255,255,0.06)", display: "inline-flex", alignItems: "center", gap: 3 }}>
            <CloudIcon size={9}/>
          </button>
        )}
        {!song.local_deleted && (
          reprocessing ? (
            <span style={{ fontSize: 10, color: "rgba(237,233,255,0.5)", maxWidth: 100, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {reprocessMsg}
            </span>
          ) : (
            <button
              onClick={handleReprocess}
              title="Re-process"
              style={{
                all: "unset", cursor: "pointer",
                fontSize: 10.5, padding: "3px 8px", borderRadius: 5,
                background: "rgba(255,255,255,0.04)",
                color: "rgba(237,233,255,0.45)",
                border: "1px solid rgba(255,255,255,0.06)",
              }}
            >↻</button>
          )
        )}
        {!song.local_deleted && (
          confirming === "choose" ? (
            <>
              <button onClick={(e) => { e.stopPropagation(); setConfirming("local"); handleDelete(e); }}
                style={{ all:"unset", cursor:"pointer", fontSize:10.5, padding:"3px 8px", borderRadius:5, background:"rgba(255,183,112,0.15)", color:"#FFB370", border:"1px solid rgba(255,183,112,0.3)" }}>
                Solo locale
              </button>
              <button onClick={(e) => { e.stopPropagation(); setConfirming("everywhere"); handleDelete(e); }}
                style={{ all:"unset", cursor:"pointer", fontSize:10.5, padding:"3px 8px", borderRadius:5, background:"rgba(242,61,109,0.15)", color:"#F23D6D", border:"1px solid rgba(242,61,109,0.3)" }}>
                Ovunque
              </button>
              <button onClick={(e) => { e.stopPropagation(); setConfirming(false); }}
                style={{ all:"unset", cursor:"pointer", fontSize:10.5, padding:"3px 8px", borderRadius:5, background:"rgba(255,255,255,0.04)", color:"rgba(237,233,255,0.45)", border:"1px solid rgba(255,255,255,0.06)" }}>
                ✕
              </button>
            </>
          ) : (
            <button
              onClick={handleDelete}
              onBlur={() => setConfirming(false)}
              style={{
                all: "unset", cursor: "pointer",
                fontSize: 10.5, padding: "3px 8px", borderRadius: 5,
                background: confirming ? "rgba(242,61,109,0.2)" : "rgba(255,255,255,0.04)",
                color: confirming ? "#F23D6D" : "rgba(237,233,255,0.45)",
                border: confirming ? "1px solid rgba(242,61,109,0.3)" : "1px solid rgba(255,255,255,0.06)",
                transition: "all 140ms",
              }}
            >
              {confirming ? "?" : "✕"}
            </button>
          )
        )}
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

function AddTrackRow({ onAdd }) {
  const [hovered, setHovered] = useState(false);
  return (
    <div
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onClick={onAdd}
      style={{
        display: "flex", alignItems: "center", gap: 12,
        padding: "8px 12px", borderRadius: 10, cursor: "pointer",
        background: hovered ? "rgba(255,107,90,0.08)" : "transparent",
        transition: "background 140ms",
      }}
    >
      <div style={{
        width: 44, height: 44, borderRadius: 8, flexShrink: 0,
        border: "1.5px dashed rgba(255,107,90,0.4)",
        display: "flex", alignItems: "center", justifyContent: "center",
        color: "#FF9070", fontSize: 20,
      }}>+</div>
      <div style={{ fontSize: 13, fontWeight: 600, color: "#FF9070" }}>Add from YouTube</div>
    </div>
  );
}

const FILTERS = [
  { id: "all",          label: "All" },
  { id: "lrc_enhanced", label: "LRC Enhanced" },
  { id: "lrc",          label: "LRC" },
  { id: "cloud",        label: "Cloud" },
  { id: "cloud_only",   label: "Cloud Only" },
];

function letterKey(title) {
  const first = (title || "").trimStart()[0] || "";
  const upper = first.toUpperCase();
  return /[A-Z]/.test(upper) ? upper : "#";
}

function CloudOrphanRow({ safeName, onRestored }) {
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  const handleRestore = async (e) => {
    e.stopPropagation();
    if (busy) return;
    setBusy(true); setMsg("Ripristino...");
    try {
      await invoke("cloud_restore_song", { safeName });
      setMsg("Fatto!");
      setTimeout(() => onRestored(), 800);
    } catch (err) {
      setMsg("Errore: " + err);
      setTimeout(() => setBusy(false), 2000);
    }
  };

  // Display a readable name from safe_name (e.g. "artist_-_title" → "artist - title")
  const displayName = safeName.replace(/_-_/g, " - ").replace(/_/g, " ");

  return (
    <div style={{
      display: "flex", alignItems: "center", gap: 12,
      padding: "8px 12px", borderRadius: 10,
      background: "rgba(34,211,164,0.04)",
      border: "1px solid rgba(34,211,164,0.12)",
      marginBottom: 4,
    }}>
      <div style={{
        width: 44, height: 44, borderRadius: 8, flexShrink: 0,
        background: "rgba(34,211,164,0.1)",
        display: "flex", alignItems: "center", justifyContent: "center",
        color: "#22D3A4",
      }}>
        <CloudIcon size={18}/>
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: "rgba(237,233,255,0.75)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {displayName}
        </div>
        <div style={{ fontSize: 11, color: "rgba(34,211,164,0.6)", marginTop: 2 }}>Solo su MEGA</div>
      </div>
      {busy ? (
        <span style={{ fontSize: 11, color: "rgba(34,211,164,0.7)" }}>{msg}</span>
      ) : (
        <button onClick={handleRestore} style={{
          all: "unset", cursor: "pointer",
          display: "inline-flex", alignItems: "center", gap: 5,
          fontSize: 11.5, fontWeight: 600, padding: "6px 14px", borderRadius: 8,
          background: "rgba(34,211,164,0.12)",
          color: "#22D3A4",
          border: "1px solid rgba(34,211,164,0.25)",
        }}>
          <CloudIcon size={11}/> Scarica
        </button>
      )}
    </div>
  );
}

export default function Library({ songs, onPlay, onDelete, onRefresh, onAddSong, onReprocess }) {
  const [q, setQ] = useState("");
  const [filter, setFilter] = useState("all");
  const [viewMode, setViewMode] = useState("list");
  const [cloudOrphans, setCloudOrphans] = useState(null); // null=not loaded, []|[...]=loaded
  const [loadingOrphans, setLoadingOrphans] = useState(false);

  const fetchCloudOrphans = useCallback(async () => {
    setLoadingOrphans(true);
    try {
      const orphans = await invoke("cloud_list_remote_songs");
      setCloudOrphans(orphans);
    } catch (e) {
      setCloudOrphans([]);
    } finally {
      setLoadingOrphans(false);
    }
  }, []);

  const filtered = useMemo(() => {
    let list = songs;
    const needle = q.trim().toLowerCase();
    if (needle) list = list.filter(s => (s.title || "").toLowerCase().includes(needle) || (s.artist || "").toLowerCase().includes(needle));
    if (filter === "lrc_enhanced") list = list.filter(s => s.lrc_enhanced);
    else if (filter === "lrc") list = list.filter(s => s.lrc && !s.lrc_enhanced);
    else if (filter === "cloud") list = list.filter(s => s.cloud_synced);
    else if (filter === "cloud_only") list = list.filter(s => s.local_deleted);
    return list;
  }, [songs, q, filter]);

  const groups = useMemo(() => {
    const sorted = [...filtered].sort((a, b) =>
      (a.title || "").localeCompare(b.title || "", "it", { sensitivity: "base" })
    );
    const map = {};
    for (const s of sorted) {
      const key = letterKey(s.title);
      (map[key] = map[key] || []).push(s);
    }
    return Object.entries(map).sort(([a], [b]) => {
      if (a === "#") return 1;
      if (b === "#") return -1;
      return a.localeCompare(b);
    });
  }, [filtered]);

  return (
    <div style={{ padding: "28px 36px 36px", overflowY: "auto", height: "100%", boxSizing: "border-box" }}>
      {/* Header */}
      <div style={{ display: "flex", alignItems: "flex-end", gap: 16, marginBottom: 24 }}>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 2, color: "#FF9070", textTransform: "uppercase", marginBottom: 8 }}>
            Your library
          </div>
          <h1 style={{ margin: 0, fontFamily: "var(--font-display)", fontWeight: 700, fontSize: 36, letterSpacing: -1.2, lineHeight: 1, color: "#FFF" }}>
            Songs
          </h1>
          <div style={{ marginTop: 8, fontSize: 13.5, color: "rgba(237,233,255,0.55)", fontWeight: 500 }}>
            {songs.length} tracks · {songs.filter(s => s.lrc_enhanced).length} word-synced · {songs.filter(s => s.lrc && !s.lrc_enhanced).length} line-synced{songs.filter(s => s.cloud_synced || s.local_deleted).length > 0 ? ` · ${songs.filter(s => s.cloud_synced || s.local_deleted).length} su MEGA` : ""}
          </div>
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <GhostBtn onClick={onRefresh} icon={<RefreshIcon/>}>Refresh</GhostBtn>
          <PrimaryBtn onClick={onAddSong} icon={<PlusIcon/>}>Add song</PrimaryBtn>
        </div>
      </div>

      {/* Search + filters + view toggle */}
      <div style={{ display: "flex", gap: 10, alignItems: "center", margin: "28px 0 18px", flexWrap: "wrap" }}>
        {/* Search */}
        <div style={{
          flex: 1, minWidth: 180, display: "flex", alignItems: "center", gap: 10,
          padding: "10px 14px", borderRadius: 12,
          background: "rgba(255,255,255,0.03)",
          border: "1px solid rgba(255,255,255,0.06)",
        }}>
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none">
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
        </div>

        {/* LRC filter pills */}
        <div style={{ display: "flex", gap: 4, padding: 4, borderRadius: 10, background: "rgba(255,255,255,0.03)", border: "1px solid rgba(255,255,255,0.06)" }}>
          {FILTERS.map(f => (
            <button key={f.id} onClick={() => setFilter(f.id)} style={{
              all: "unset", cursor: "pointer",
              padding: "6px 12px", borderRadius: 7,
              fontSize: 12, fontWeight: 600,
              color: filter === f.id ? "#FFF" : "rgba(237,233,255,0.55)",
              background: filter === f.id
                ? (f.id === "lrc_enhanced" ? CK_GRADIENT : "rgba(255,255,255,0.08)")
                : "transparent",
              transition: "all 140ms",
            }}>{f.label}</button>
          ))}
        </div>

        {/* View mode toggle */}
        <div style={{ display: "flex", gap: 2, padding: 4, borderRadius: 10, background: "rgba(255,255,255,0.03)", border: "1px solid rgba(255,255,255,0.06)" }}>
          <button
            onClick={() => setViewMode("grid")}
            title="Grid view"
            style={{
              all: "unset", cursor: "pointer",
              padding: "6px 10px", borderRadius: 7,
              color: viewMode === "grid" ? "#FFF" : "rgba(237,233,255,0.45)",
              background: viewMode === "grid" ? "rgba(255,255,255,0.08)" : "transparent",
              transition: "all 140ms", display: "flex", alignItems: "center",
            }}
          >
            <GridIcon/>
          </button>
          <button
            onClick={() => setViewMode("list")}
            title="List view"
            style={{
              all: "unset", cursor: "pointer",
              padding: "6px 10px", borderRadius: 7,
              color: viewMode === "list" ? "#FFF" : "rgba(237,233,255,0.45)",
              background: viewMode === "list" ? "rgba(255,255,255,0.08)" : "transparent",
              transition: "all 140ms", display: "flex", alignItems: "center",
            }}
          >
            <ListIcon/>
          </button>
        </div>
      </div>

      {/* Content */}
      {filtered.length === 0 ? (
        <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", padding: 48, color: "rgba(237,233,255,0.4)", textAlign: "center", gap: 12 }}>
          <div style={{ fontSize: 48, opacity: 0.5 }}>🎶</div>
          <div style={{ fontSize: 14 }}>No songs yet. Head to Download to add some.</div>
        </div>
      ) : viewMode === "grid" ? (
        <div>
          {groups.map(([letter, groupSongs]) => (
            <div key={letter} style={{ marginBottom: 28 }}>
              <LetterHeader letter={letter}/>
              <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(200px, 1fr))", gap: 16 }}>
                {groupSongs.map(s => (
                  <TrackCard key={s._dir} song={s} onPlay={onPlay} onDelete={onDelete} onReprocess={onReprocess}/>
                ))}
              </div>
            </div>
          ))}
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(200px, 1fr))", gap: 16, marginTop: groups.length > 0 ? 0 : 0 }}>
            <AddTrackCard onAdd={onAddSong}/>
          </div>
        </div>
      ) : (
        <div>
          {groups.map(([letter, groupSongs]) => (
            <div key={letter} style={{ marginBottom: 20 }}>
              <LetterHeader letter={letter}/>
              <div style={{ display: "flex", flexDirection: "column" }}>
                {groupSongs.map(s => (
                  <TrackRow key={s._dir} song={s} onPlay={onPlay} onDelete={onDelete} onReprocess={onReprocess}/>
                ))}
              </div>
            </div>
          ))}
          <AddTrackRow onAdd={onAddSong}/>
        </div>
      )}

      {/* Cloud orphans section */}
      <div style={{ marginTop: 32 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 10 }}>
          <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 1.4, color: "#22D3A4", textTransform: "uppercase" }}>
            Solo su MEGA
          </div>
          <button
            onClick={fetchCloudOrphans}
            disabled={loadingOrphans}
            style={{
              all: "unset", cursor: "pointer",
              fontSize: 10.5, padding: "3px 10px", borderRadius: 6,
              background: "rgba(34,211,164,0.08)",
              color: "#22D3A4",
              border: "1px solid rgba(34,211,164,0.2)",
            }}
          >
            {loadingOrphans ? "Carico..." : cloudOrphans === null ? "Controlla MEGA" : "↻ Aggiorna"}
          </button>
        </div>
        {cloudOrphans !== null && (
          cloudOrphans.length === 0 ? (
            <div style={{ fontSize: 12.5, color: "rgba(237,233,255,0.35)", padding: "8px 0" }}>
              Nessuna canzone orphan su MEGA.
            </div>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              {cloudOrphans.map((name) => (
                <CloudOrphanRow
                  key={name}
                  safeName={name}
                  onRestored={() => {
                    setCloudOrphans((prev) => prev.filter((n) => n !== name));
                    onRefresh();
                  }}
                />
              ))}
            </div>
          )
        )}
      </div>
    </div>
  );
}

function LetterHeader({ letter }) {
  return (
    <div style={{
      fontSize: 11, fontWeight: 700, letterSpacing: 1.5,
      color: "#FF9070", textTransform: "uppercase",
      padding: "4px 0 10px",
      borderBottom: "1px solid rgba(255,255,255,0.05)",
      marginBottom: 12,
    }}>
      {letter}
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
function GridIcon() {
  return <svg width="15" height="15" viewBox="0 0 24 24" fill="none">
    <rect x="3" y="3" width="7" height="7" rx="1.5" fill="currentColor"/>
    <rect x="14" y="3" width="7" height="7" rx="1.5" fill="currentColor"/>
    <rect x="3" y="14" width="7" height="7" rx="1.5" fill="currentColor"/>
    <rect x="14" y="14" width="7" height="7" rx="1.5" fill="currentColor"/>
  </svg>;
}
function ListIcon() {
  return <svg width="15" height="15" viewBox="0 0 24 24" fill="none">
    <rect x="3" y="4" width="18" height="2.5" rx="1.25" fill="currentColor"/>
    <rect x="3" y="10.75" width="18" height="2.5" rx="1.25" fill="currentColor"/>
    <rect x="3" y="17.5" width="18" height="2.5" rx="1.25" fill="currentColor"/>
  </svg>;
}
