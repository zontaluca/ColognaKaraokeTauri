import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import Sidebar from "./components/Sidebar.jsx";
import Library from "./views/Library.jsx";
import Download from "./views/Download.jsx";
import Player from "./views/Player.jsx";
import Leaderboard from "./views/Leaderboard.jsx";
import { JobsProvider, JobsToast } from "./jobsContext.jsx";
import Background from "./components/Background.jsx";

export default function App() {
  const [view, setView] = useState("library");
  const [songs, setSongs] = useState([]);
  const [currentSong, setCurrentSong] = useState(null);

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
    setView("player");
  };

  const deleteSong = (dir) => {
    setSongs((prev) => prev.filter((s) => s._dir !== dir));
    if (currentSong?._dir === dir) setCurrentSong(null);
  };

  const onJobDone = useCallback(() => { refreshLibrary(); }, [refreshLibrary]);

  return (
    <JobsProvider onJobDone={onJobDone}>
      <Background view={view} />
      <div className="app">
        <Sidebar view={view} onView={setView} />
        <main className="main">
          {view === "library" && <Library songs={songs} onPlay={playSong} onDelete={deleteSong} onRefresh={refreshLibrary} />}
          {view === "download" && <Download />}
          {view === "player" && <Player song={currentSong} />}
          {view === "leaderboard" && <Leaderboard songs={songs} />}
        </main>
      </div>
      <JobsToast />
    </JobsProvider>
  );
}
