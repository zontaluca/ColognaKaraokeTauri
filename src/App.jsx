import { useCallback, useEffect, useRef, useState } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";

import Sidebar from "./components/Sidebar.jsx";
import Library from "./views/Library.jsx";
import Download from "./views/Download.jsx";
import Player from "./views/Player.jsx";
import Leaderboard from "./views/Leaderboard.jsx";
import Settings from "./views/Settings.jsx";
import { JobsProvider, JobsToast } from "./jobsContext.jsx";
import Background from "./components/Background.jsx";

const CK_GRADIENT = "linear-gradient(135deg, #FFB370 0%, #FF6B5A 40%, #F23D6D 100%)";

function MiniPlayer({ song, isPlaying, presentOpen, playerRef, onNavigateToPlayer }) {
  if (!song) return null;

  const coverSrc = song.cover_path ? convertFileSrc(song.cover_path) : null;

  return (
    <div style={{
      position: "fixed", bottom: 0, left: 240, right: 0, zIndex: 100,
      display: "flex", alignItems: "center", gap: 14,
      padding: "10px 20px",
      background: "rgba(7,6,12,0.85)",
      backdropFilter: "blur(20px)",
      borderTop: "1px solid rgba(255,255,255,0.07)",
      boxShadow: "0 -8px 32px rgba(0,0,0,0.4)",
    }}>
      {/* Cover + song info */}
      <button onClick={onNavigateToPlayer} style={{
        all: "unset", cursor: "pointer",
        display: "flex", alignItems: "center", gap: 10, flex: "0 0 auto",
      }}>
        <div style={{
          width: 38, height: 38, borderRadius: 8, flexShrink: 0,
          background: coverSrc ? `url(${coverSrc}) center/cover no-repeat` : "linear-gradient(135deg, #FF9A76, #FF4F76)",
          boxShadow: "0 4px 12px rgba(0,0,0,0.4)",
        }}/>
        <div style={{ maxWidth: 180 }}>
          <div style={{ fontSize: 12.5, fontWeight: 600, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {song.title || "Unknown"}
          </div>
          <div style={{ fontSize: 11, color: "rgba(237,233,255,0.5)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {song.artist || "Unknown artist"}
          </div>
        </div>
      </button>

      {/* Play/Pause */}
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

      {/* Stop */}
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

      {/* Presentation toggle */}
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

  const refreshLibrary = useCallback(async () => {
    try {
      const list = await invoke("scan_library");
      setSongs(list);
    } catch (e) {
      console.error("scan_library failed", e);
    }
  }, []);

  useEffect(() => {
    refreshLibrary();
  }, [refreshLibrary]);

  const playSong = (song) => {
    setCurrentSong(song);
    setIsPlaying(false);
    setView("player");
  };

  const deleteSong = (dir) => {
    setSongs((prev) => prev.filter((s) => s._dir !== dir));
    if (currentSong?._dir === dir) setCurrentSong(null);
  };

  const onJobDone = useCallback(() => { refreshLibrary(); }, [refreshLibrary]);

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

  const showMiniPlayer = view !== "player" && currentSong != null;

  return (
    <JobsProvider onJobDone={onJobDone}>
      <Background view={view} />
      <div className="app">
        <Sidebar view={view} onView={setView} currentSong={currentSong} isPlaying={isPlaying} />
        <main className="main" style={showMiniPlayer ? { paddingBottom: 64 } : undefined}>
          {view === "library" && <Library songs={songs} onPlay={playSong} onDelete={deleteSong} onRefresh={refreshLibrary} onAddSong={() => setView("download")} onReprocess={refreshCurrentSong}/>}
          {view === "download" && <Download />}
          {view === "leaderboard" && <Leaderboard songs={songs} />}
          {view === "settings" && <Settings />}
          {/* Player always mounted to preserve audio state across view changes */}
          <div style={{ display: view === "player" ? "contents" : "none", height: "100%" }}>
            <Player
              ref={playerRef}
              song={currentSong}
              onPlayingChange={setIsPlaying}
              presentOpen={presentOpen}
              onPresentOpenChange={setPresentOpen}
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
        />
      )}
      <JobsToast />
    </JobsProvider>
  );
}
