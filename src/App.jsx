import { useCallback, useEffect, useRef, useState } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";

import Sidebar from "./components/Sidebar.jsx";
import Library from "./views/Library.jsx";
import Download from "./views/Download.jsx";
import Player from "./views/Player.jsx";
import Players from "./views/Players.jsx";
import Leaderboard from "./views/Leaderboard.jsx";
import Settings from "./views/Settings.jsx";
import { JobsProvider, JobsToast } from "./jobsContext.jsx";
import { CloudProvider, CloudToast } from "./cloudContext.jsx";
import Background from "./components/Background.jsx";

const CK_GRADIENT = "linear-gradient(135deg, #FFB370 0%, #FF6B5A 40%, #F23D6D 100%)";

// ─── Mini player bar ─────────────────────────────────────────────────────────
function MiniPlayer({ song, isPlaying, presentOpen, playerRef, onNavigateToPlayer, queueActive, readyEntry, onConfirm, onStop }) {
  if (!song && !readyEntry) return null;

  const coverSrc = song?.cover_path ? convertFileSrc(song.cover_path) : null;

  return (
    <div style={{
      position: "fixed", bottom: 0, left: 240, right: 0, zIndex: 200,
      display: "flex", alignItems: "center", gap: 14,
      padding: "10px 20px",
      background: "rgba(7,6,12,0.92)",
      backdropFilter: "blur(20px)",
      borderTop: "1px solid rgba(255,255,255,0.07)",
      boxShadow: "0 -8px 32px rgba(0,0,0,0.4)",
    }}>
      {/* Current song info */}
      <button onClick={onNavigateToPlayer} style={{
        all: "unset", cursor: "pointer",
        display: "flex", alignItems: "center", gap: 10, flex: "0 0 auto",
      }}>
        <div style={{
          width: 38, height: 38, borderRadius: 8, flexShrink: 0,
          background: coverSrc ? `url(${coverSrc}) center/cover no-repeat` : "linear-gradient(135deg, #FF9A76, #FF4F76)",
          boxShadow: "0 4px 12px rgba(0,0,0,0.4)",
        }}/>
        <div style={{ maxWidth: 160 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {song.title || "Unknown"}
          </div>
          <div style={{ fontSize: 11, color: "rgba(237,233,255,0.5)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {song.artist || "Unknown artist"}
          </div>
        </div>
      </button>

      <button onClick={() => playerRef.current?.toggle()} style={{
        all: "unset", cursor: "pointer", flexShrink: 0,
        width: 38, height: 38, borderRadius: "50%",
        background: CK_GRADIENT, color: "#FFF",
        display: "flex", alignItems: "center", justifyContent: "center",
        boxShadow: "0 6px 16px rgba(242,61,109,0.4)",
      }}>
        {isPlaying
          ? <svg width="14" height="14" viewBox="0 0 24 24"><rect x="6" y="5" width="4" height="14" fill="#FFF" rx="1"/><rect x="14" y="5" width="4" height="14" fill="#FFF" rx="1"/></svg>
          : <svg width="14" height="14" viewBox="0 0 24 24"><path d="M7 4.5v15L20 12 7 4.5z" fill="#FFF"/></svg>
        }
      </button>

      <button onClick={() => playerRef.current?.stop()} style={{
        all: "unset", cursor: "pointer", flexShrink: 0,
        width: 30, height: 30, borderRadius: 8,
        display: "flex", alignItems: "center", justifyContent: "center",
        background: "rgba(255,255,255,0.05)",
        border: "1px solid rgba(255,255,255,0.08)",
        color: "#EDE9FF",
      }}>
        <svg width="10" height="10" viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" fill="currentColor" rx="1"/></svg>
      </button>

      <button onClick={() => playerRef.current?.openPresentation()} style={{
        all: "unset", cursor: "pointer", flexShrink: 0,
        display: "flex", alignItems: "center", gap: 6,
        padding: "6px 12px", borderRadius: 999,
        fontSize: 11.5, fontWeight: 600,
        color: presentOpen ? "#FFF" : "rgba(237,233,255,0.5)",
        background: presentOpen ? "rgba(255,107,90,0.18)" : "rgba(255,255,255,0.04)",
        border: presentOpen ? "1px solid rgba(255,107,90,0.35)" : "1px solid rgba(255,255,255,0.06)",
        transition: "all 140ms",
      }}>
        <span style={{
          width: 7, height: 7, borderRadius: "50%",
          background: presentOpen ? "#FF6B5A" : "rgba(255,255,255,0.2)",
          boxShadow: presentOpen ? "0 0 8px #FF6B5A" : "none",
        }}/>
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none">
          <rect x="2" y="3" width="20" height="14" rx="2" stroke="currentColor" strokeWidth="1.8"/>
          <path d="M8 21h8M12 17v4" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round"/>
        </svg>
        {presentOpen ? "Presentation ON" : "Presentation"}
      </button>

      {/* Pronto! — shown when next player is waiting and user is NOT in player view */}
      {readyEntry && (
        <>
          <div style={{ width: 1, alignSelf: "stretch", background: "rgba(255,107,90,0.3)", margin: "0 6px" }}/>
          <div style={{ minWidth: 0, flex: 1 }}>
            <div style={{ fontSize: 10, fontWeight: 700, letterSpacing: 0.8, textTransform: "uppercase", color: "#FF9070", marginBottom: 1 }}>Prossimo</div>
            <div style={{ fontSize: 12.5, fontWeight: 700, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {readyEntry.player_name || "Player"} · {readyEntry.song?.title || "—"}
            </div>
          </div>
          <button onClick={onConfirm} style={{
            all: "unset", cursor: "pointer", flexShrink: 0,
            padding: "8px 16px", borderRadius: 10,
            background: CK_GRADIENT, color: "#FFF",
            fontSize: 12.5, fontWeight: 800,
            boxShadow: "0 4px 14px rgba(242,61,109,0.4)",
            display: "flex", alignItems: "center", gap: 6, transition: "transform 120ms",
          }}
          onMouseEnter={e => e.currentTarget.style.transform = "scale(1.04)"}
          onMouseLeave={e => e.currentTarget.style.transform = "scale(1)"}
          >
            Pronto!
            <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor"><path d="M5 4l14 8-14 8V4z"/></svg>
          </button>
        </>
      )}

      {/* Stop queue button */}
      {queueActive && (
        <button onClick={onStop} style={{
          all: "unset", cursor: "pointer", flexShrink: 0,
          display: "flex", alignItems: "center", gap: 5,
          padding: "5px 10px", borderRadius: 7,
          fontSize: 11, fontWeight: 700, color: "rgba(237,233,255,0.35)",
          background: "rgba(255,255,255,0.04)",
          border: "1px solid rgba(255,255,255,0.07)",
          transition: "all 120ms",
        }}
        onMouseEnter={e => { e.currentTarget.style.color = "#F23D6D"; e.currentTarget.style.background = "rgba(242,61,109,0.1)"; e.currentTarget.style.borderColor = "rgba(242,61,109,0.25)"; }}
        onMouseLeave={e => { e.currentTarget.style.color = "rgba(237,233,255,0.35)"; e.currentTarget.style.background = "rgba(255,255,255,0.04)"; e.currentTarget.style.borderColor = "rgba(255,255,255,0.07)"; }}
        >
          <svg width="9" height="9" viewBox="0 0 24 24" fill="none">
            <rect x="6" y="6" width="12" height="12" rx="1" fill="currentColor"/>
          </svg>
          Ferma coda
        </button>
      )}
    </div>
  );
}


export default function App() {
  const [view, setView] = useState("library");
  const [songs, setSongs] = useState([]);
  const [currentSong, setCurrentSong] = useState(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [presentOpen, setPresentOpen] = useState(false);
  const playerRef = useRef(null);

  // Party queue state
  const [queue, setQueue]           = useState([]);
  const [queueIdx, setQueueIdx]     = useState(-1);
  const [readyEntry, setReadyEntry] = useState(null);
  const queueRef          = useRef([]);
  const queueIdxRef       = useRef(-1);
  const queueSaveTimerRef = useRef(null);
  const queueRestoredRef  = useRef(false);
  useEffect(() => { queueRef.current    = queue;    }, [queue]);
  useEffect(() => { queueIdxRef.current = queueIdx; }, [queueIdx]);

  const refreshLibrary = useCallback(async () => {
    try {
      const list = await invoke("scan_library");
      setSongs(list);
    } catch (e) {
      console.error("scan_library failed", e);
    }
  }, []);

  useEffect(() => { refreshLibrary(); }, [refreshLibrary]);

  const playSong = useCallback((song) => {
    setCurrentSong(song);
    setIsPlaying(false);
    setView("player");
  }, []);

  const deleteSong = (dir) => {
    setSongs((prev) => prev.filter((s) => s._dir !== dir));
    if (currentSong?._dir === dir) setCurrentSong(null);
  };

  const onJobDone        = useCallback(() => { refreshLibrary(); }, [refreshLibrary]);
  const onCloudSyncDone  = useCallback(() => { refreshLibrary(); }, [refreshLibrary]);

  const refreshCurrentSong = useCallback(async () => {
    try {
      const list = await invoke("scan_library");
      setSongs(list);
      if (currentSong) {
        const updated = list.find((s) => s._dir === currentSong._dir);
        if (updated) setCurrentSong(updated);
      }
    } catch (e) {
      console.error("refreshCurrentSong failed", e);
    }
  }, [currentSong]);

  // Persist active queue whenever it changes
  useEffect(() => {
    if (queue.length === 0) {
      // Clear immediately — no debounce so closing app right after completion doesn't restore
      invoke("active_queue_save", { state: null }).catch(console.error);
      return;
    }
    if (queueSaveTimerRef.current) clearTimeout(queueSaveTimerRef.current);
    queueSaveTimerRef.current = setTimeout(() => {
      const state = {
        entries: queue.map(({ id, player_name, song_dir }) => ({ id, player_name, song_dir })),
        queue_idx: queueIdx,
      };
      invoke("active_queue_save", { state }).catch(console.error);
    }, 400);
  }, [queue, queueIdx]);

  // Restore active queue once after the first library load
  useEffect(() => {
    if (songs.length === 0 || queueRestoredRef.current) return;
    queueRestoredRef.current = true;
    invoke("active_queue_load").then(saved => {
      if (!saved || !Array.isArray(saved.entries) || saved.entries.length === 0) return;
      const resolved = saved.entries
        .map(e => ({ ...e, song: songs.find(s => s._dir === e.song_dir) || null }))
        .filter(e => e.song !== null);
      if (resolved.length === 0) return;
      const idx = Math.min(Math.max(saved.queue_idx ?? 0, 0), resolved.length - 1);
      setQueue(resolved);
      setQueueIdx(idx);
      setCurrentSong(resolved[idx].song);
    }).catch(() => {});
  }, [songs]);

  // ── Queue management ────────────────────────────────────────────────────────

  const startQueue = useCallback((entries) => {
    invoke("players_save", { players: [] }).catch(console.error);
    setQueue(entries);
    setQueueIdx(0);
    playSong(entries[0].song);
  }, [playSong]);

  const addToQueue = useCallback((entry) => {
    setQueue(prev => [...prev, entry]);
  }, []);

  const onSongEnd = useCallback(() => {
    const q   = queueRef.current;
    const idx = queueIdxRef.current;
    if (q.length === 0 || idx < 0) return;
    const nextIdx = idx + 1;
    if (nextIdx < q.length) {
      const next = q[nextIdx];
      setReadyEntry(next);
      emit("karaoke://presentation-next", {
        player_name: next.player_name,
        song: next.song ? { title: next.song.title, artist: next.song.artist, cover_path: next.song.cover_path } : null,
      }).catch(() => {});
    } else {
      emit("karaoke://presentation-next", null).catch(() => {});
      setQueue([]);
      setQueueIdx(-1);
    }
  }, []);

  const confirmNext = useCallback(() => {
    if (!readyEntry) return;
    const nextIdx = queueIdxRef.current + 1;
    const next = readyEntry;
    setReadyEntry(null);
    setQueueIdx(nextIdx);
    playerRef.current?.queuePlay();
    playSong(next.song);
  }, [readyEntry, playSong]);

  const stopQueue = useCallback(() => {
    emit("karaoke://presentation-next", null).catch(() => {});
    setQueue([]);
    setQueueIdx(-1);
    setReadyEntry(null);
  }, []);

  const clearPastFromQueue = useCallback(() => {
    const pastCount = queueIdxRef.current;
    if (pastCount <= 0) return;
    setQueue(prev => prev.slice(pastCount));
    setQueueIdx(0);
  }, []);

  const removeFromQueue = useCallback((id) => {
    const q = queueRef.current;
    const currentIdx = queueIdxRef.current;
    const idx = q.findIndex(e => e.id === id);
    if (idx < 0 || idx === currentIdx) return;
    setQueue(prev => prev.filter(e => e.id !== id));
    if (idx < currentIdx) setQueueIdx(currentIdx - 1);
  }, []);

  // ── Layout ──────────────────────────────────────────────────────────────────

  const showMiniPlayer = view !== "player" && (currentSong != null || readyEntry != null);
  const queueActive = queue.length > 0;

  return (
    <JobsProvider onJobDone={onJobDone}>
    <CloudProvider onSyncDone={onCloudSyncDone}>
      <Background view={view} />
      <div className="app">
        <Sidebar view={view} onView={setView} currentSong={currentSong} isPlaying={isPlaying} />
        <main className="main" style={showMiniPlayer ? { paddingBottom: 64 } : undefined}>
          {view === "library"     && <Library songs={songs} onPlay={playSong} onDelete={deleteSong} onRefresh={refreshLibrary} onAddSong={() => setView("download")} onReprocess={refreshCurrentSong}/>}
          {view === "download"    && <Download />}
          {view === "players"     && <Players songs={songs} onStartQueue={startQueue} onAddToQueue={addToQueue} activeQueue={queue} queueIdx={queueIdx} onClearPastFromQueue={clearPastFromQueue} onRemoveFromQueue={removeFromQueue} />}
          {view === "leaderboard" && <Leaderboard songs={songs} />}
          {view === "settings"    && <Settings />}
          <div style={{ display: view === "player" ? "contents" : "none", height: "100%" }}>
            <Player
              ref={playerRef}
              song={currentSong}
              onPlayingChange={setIsPlaying}
              presentOpen={presentOpen}
              onPresentOpenChange={setPresentOpen}
              onSongEnd={onSongEnd}
              readyEntry={readyEntry}
              onConfirmNext={confirmNext}
              onStopQueue={stopQueue}
            />
          </div>
        </main>
      </div>

      {showMiniPlayer && (
        <MiniPlayer
          song={currentSong}
          isPlaying={isPlaying}
          presentOpen={presentOpen}
          playerRef={playerRef}
          onNavigateToPlayer={() => setView("player")}
          queueActive={queueActive}
          readyEntry={readyEntry}
          onConfirm={confirmNext}
          onStop={stopQueue}
        />
      )}

      <JobsToast />
      <CloudToast />
    </CloudProvider>
    </JobsProvider>
  );
}
