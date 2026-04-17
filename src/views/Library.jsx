import { useMemo, useState } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";

function formatDuration(sec) {
  const s = Math.max(0, Number(sec) || 0);
  const m = Math.floor(s / 60);
  const r = s % 60;
  return `${m}:${String(r).padStart(2, "0")}`;
}

function SongCard({ song, onPlay, onDelete }) {
  const hasLrc = Boolean(song.lrc);
  const [confirming, setConfirming] = useState(false);

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

  const coverSrc = song.cover_path ? convertFileSrc(song.cover_path) : null;
  return (
    <div className="song-card" onClick={() => onPlay(song)}>
      <div className="cover">
        {coverSrc ? <img src={coverSrc} alt="" /> : "🎵"}
      </div>
      <h3>{song.title || "Unknown"}</h3>
      <div className="meta">
        <span>{song.artist || "Unknown"}</span>
        <span>
          {formatDuration(song.duration_sec)}
          {hasLrc && <span className="lrc-badge">LRC</span>}
        </span>
      </div>
      <button
        className="btn btn-secondary"
        style={{ marginTop: 8, fontSize: 12, padding: "3px 10px", color: confirming ? "#FF3333" : undefined }}
        onClick={handleDelete}
        onBlur={() => setConfirming(false)}
      >
        {confirming ? "Confirm delete?" : "Delete"}
      </button>
    </div>
  );
}

export default function Library({ songs, onPlay, onDelete, onRefresh }) {
  const [q, setQ] = useState("");

  const filtered = useMemo(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return songs;
    return songs.filter((s) => {
      const t = (s.title || "").toLowerCase();
      const a = (s.artist || "").toLowerCase();
      return t.includes(needle) || a.includes(needle);
    });
  }, [songs, q]);

  return (
    <>
      <div className="main-header">
        <div>
          <h2>Featured Songs</h2>
          <p>{songs.length} tracks in your library</p>
        </div>
        <button className="btn btn-secondary" onClick={onRefresh}>🔄 Refresh</button>
      </div>

      <div className="search-bar">
        <span>🔍</span>
        <input
          placeholder="Search songs, artists..."
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
      </div>

      <div className="content">
        {filtered.length === 0 ? (
          <div className="empty">
            <div className="icon">🎶</div>
            <div>No songs yet. Head to Download to add some.</div>
          </div>
        ) : (
          <div className="grid">
            {filtered.map((s) => (
              <SongCard key={s._dir} song={s} onPlay={onPlay} onDelete={onDelete} />
            ))}
          </div>
        )}
      </div>
    </>
  );
}
