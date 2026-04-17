import { useEffect, useMemo, useRef, useState, useCallback } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

function formatTime(sec) {
  const s = Math.max(0, Math.floor(sec || 0));
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

function parseLrc(text) {
  if (!text) return [];
  const re = /\[(\d{2}):(\d{2})\.(\d{2,3})\]\s*(.*)/;
  return text
    .split("\n")
    .map((l) => {
      const m = l.trim().match(re);
      if (!m) return null;
      const mm = parseInt(m[1], 10);
      const ss = parseInt(m[2], 10);
      const frac = m[3];
      const ms = frac.length === 2 ? parseInt(frac, 10) * 10 : parseInt(frac, 10);
      const t = m[4].trim();
      if (!t) return null;
      return { ts_ms: (mm * 60 + ss) * 1000 + ms, text: t };
    })
    .filter(Boolean)
    .sort((a, b) => a.ts_ms - b.ts_ms);
}

function isSynced(text) {
  return /\[\d{2}:\d{2}\.\d{2,3}\]/.test(text || "");
}

function useBars(song, count = 64) {
  return useMemo(() => {
    const seed = (song?.title || "x")
      .split("")
      .reduce((acc, c) => (acc * 31 + c.charCodeAt(0)) >>> 0, 7);
    let s = seed || 1;
    const next = () => {
      s = (s * 1103515245 + 12345) & 0x7fffffff;
      return s / 0x7fffffff;
    };
    return Array.from({ length: count }, () => 30 + next() * 70);
  }, [song, count]);
}

export default function Player({ song }) {
  const audioRef = useRef(null);
  const waveformRef = useRef(null);
  const rafRef = useRef(0);
  const activeLineRef = useRef(null);
  const lastSeekRef = useRef(0);

  const [src, setSrc] = useState(null);
  const [playing, setPlaying] = useState(false);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(0.7);
  const [preferInstrumental, setPreferInstrumental] = useState(true);
  const [wordTimestamps, setWordTimestamps] = useState(null);
  const [currentIdx, setCurrentIdx] = useState(-1);
  const [wordIdx, setWordIdx] = useState(-1);
  const [displayTime, setDisplayTime] = useState(0);

  const [challenge, setChallenge] = useState(false);
  const [playerName, setPlayerName] = useState("");
  const [askName, setAskName] = useState(false);
  const [sessionId, setSessionId] = useState(null);
  const [scoreState, setScoreState] = useState({ hits: 0, partials: 0, misses: 0 });
  const [wordStatuses, setWordStatuses] = useState({}); // word_idx -> 'hit'|'partial'|'miss'
  const [finalScore, setFinalScore] = useState(null);
  const [rank, setRank] = useState(null);

  const lrcLines = useMemo(() => parseLrc(song?.lrc || ""), [song]);
  const synced = isSynced(song?.lrc || "");
  const bars = useBars(song);

  const wordsByLine = useMemo(() => {
    if (!wordTimestamps || lrcLines.length === 0) return null;
    const buckets = lrcLines.map(() => []);
    for (const w of wordTimestamps) {
      for (let i = 0; i < lrcLines.length; i++) {
        const start = lrcLines[i].ts_ms;
        const end = lrcLines[i + 1]?.ts_ms ?? (start + 8000);
        if (w.start_ms >= start && w.start_ms < end) {
          buckets[i].push(w);
          break;
        }
      }
    }
    return buckets;
  }, [wordTimestamps, lrcLines]);

  useEffect(() => {
    setWordTimestamps(null);
    if (!song?._dir) return;
    invoke("get_words", { dir: song._dir })
      .then((words) => { if (Array.isArray(words) && words.length > 0) setWordTimestamps(words); })
      .catch(() => {});
  }, [song]);

  useEffect(() => {
    setSrc(null);
    setDuration(song?.duration_sec || 0);
    setCurrentIdx(-1);
    setWordIdx(-1);
    setDisplayTime(0);
    setWordStatuses({});
    setFinalScore(null);
    setRank(null);
    if (!song?._dir) return;
    (async () => {
      try {
        const path = await invoke("get_song_audio_path", {
          dir: song._dir,
          preferInstrumental,
        });
        setSrc(convertFileSrc(path));
      } catch (e) {
        console.error("get_song_audio_path", e);
      }
    })();
  }, [song, preferInstrumental]);

  useEffect(() => {
    const a = audioRef.current;
    if (a) a.volume = volume;
  }, [volume]);

  // rAF-driven index + progress updater
  const tick = useCallback(() => {
    const a = audioRef.current;
    if (!a) return;
    const t = a.currentTime;
    const tMs = t * 1000;

    // progress CSS var (cheap style write)
    if (waveformRef.current && duration > 0) {
      waveformRef.current.style.setProperty("--progress", String(t / duration));
    }

    // line index (binary-ish linear scan — fine for typical sizes)
    let newLine = -1;
    if (synced) {
      for (let i = 0; i < lrcLines.length; i++) {
        if (lrcLines[i].ts_ms <= tMs) newLine = i;
        else break;
      }
    }

    let newWord = -1;
    if (newLine >= 0) {
      if (wordsByLine && wordsByLine[newLine]?.length > 0) {
        const arr = wordsByLine[newLine];
        for (let i = 0; i < arr.length; i++) {
          if (arr[i].start_ms <= tMs) newWord = i;
          else break;
        }
      } else {
        const line = lrcLines[newLine];
        const words = line.text.trim().split(/\s+/);
        const lineStart = line.ts_ms;
        const lineEnd = lrcLines[newLine + 1]?.ts_ms ?? (lineStart + 3000);
        const elapsed = tMs - lineStart;
        const dur = Math.max(1, lineEnd - lineStart);
        const idx = Math.floor((elapsed / dur) * words.length);
        newWord = Math.min(Math.max(idx, 0), words.length - 1);
      }
    }

    setCurrentIdx((prev) => (prev !== newLine ? newLine : prev));
    setWordIdx((prev) => (prev !== newWord ? newWord : prev));

    // 4Hz throttle for display time (mm:ss only needs coarse updates)
    if (Math.abs(t - lastSeekRef.current) > 0.25) {
      lastSeekRef.current = t;
      setDisplayTime(t);
    }

    rafRef.current = requestAnimationFrame(tick);
  }, [synced, lrcLines, wordsByLine, duration]);

  useEffect(() => {
    if (playing) {
      rafRef.current = requestAnimationFrame(tick);
      return () => cancelAnimationFrame(rafRef.current);
    }
    cancelAnimationFrame(rafRef.current);
  }, [playing, tick]);

  useEffect(() => {
    const a = audioRef.current;
    if (!a) return;
    const onDur = () => setDuration(a.duration || song?.duration_sec || 0);
    const onEnd = () => {
      setPlaying(false);
      if (challenge && sessionId) endChallenge();
    };
    a.addEventListener("loadedmetadata", onDur);
    a.addEventListener("ended", onEnd);
    return () => {
      a.removeEventListener("loadedmetadata", onDur);
      a.removeEventListener("ended", onEnd);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [src, song, challenge, sessionId]);

  // auto-scroll active line to center
  useEffect(() => {
    if (!activeLineRef.current) return;
    activeLineRef.current.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [currentIdx]);

  // Challenge: score-tick listener
  useEffect(() => {
    if (!challenge) return;
    let unlisten;
    (async () => {
      unlisten = await listen("karaoke://score-tick", (ev) => {
        const { word_idx, status } = ev.payload || {};
        if (typeof word_idx !== "number") return;
        setWordStatuses((prev) => ({ ...prev, [word_idx]: status }));
        setScoreState((prev) => {
          const next = { ...prev };
          if (status === "hit") next.hits += 1;
          else if (status === "partial") next.partials += 1;
          else next.misses += 1;
          return next;
        });
      });
    })();
    return () => unlisten && unlisten();
  }, [challenge]);

  const toggle = async () => {
    const a = audioRef.current;
    if (!a) return;
    if (playing) {
      a.pause();
      setPlaying(false);
      return;
    }
    if (challenge && !sessionId) {
      setAskName(true);
      return;
    }
    try {
      await a.play();
      setPlaying(true);
    } catch (e) {
      console.error(e);
    }
  };

  const stop = () => {
    const a = audioRef.current;
    if (!a) return;
    a.pause();
    a.currentTime = 0;
    setPlaying(false);
    if (challenge && sessionId) endChallenge();
  };

  const seek = (pct) => {
    const a = audioRef.current;
    if (!a || !duration) return;
    a.currentTime = duration * pct;
    setDisplayTime(a.currentTime);
  };

  const startChallenge = async () => {
    if (!song?._dir || !playerName.trim()) return;
    const sid = `${Date.now()}`;
    setSessionId(sid);
    setScoreState({ hits: 0, partials: 0, misses: 0 });
    setWordStatuses({});
    try {
      await invoke("recorder_start", { songDir: song._dir, sessionId: sid });
      await invoke("pitch_start", { songDir: song._dir });
    } catch (e) {
      console.error("challenge start failed", e);
      alert("Mic start failed: " + e);
      setSessionId(null);
      return;
    }
    setAskName(false);
    try {
      await audioRef.current?.play();
      setPlaying(true);
    } catch (e) { console.error(e); }
  };

  const endChallenge = async () => {
    try { await invoke("pitch_stop"); } catch {}
    try { await invoke("recorder_stop"); } catch {}
    const total = scoreState.hits + scoreState.partials + scoreState.misses;
    const score = total === 0 ? 0 : Math.round(((scoreState.hits + 0.5 * scoreState.partials) / total) * 100);
    setFinalScore(score);
    try {
      await invoke("leaderboard_insert", {
        entry: {
          song_dir: song._dir,
          song_title: song.title || "",
          player_name: playerName.trim() || "Anonymous",
          score,
          hits: scoreState.hits,
          partials: scoreState.partials,
          misses: scoreState.misses,
        },
      });
      const top = await invoke("leaderboard_top", { songDir: song._dir, limit: 10 });
      if (Array.isArray(top)) {
        const r = top.findIndex((e) => e.score === score && e.player_name === (playerName.trim() || "Anonymous"));
        setRank(r >= 0 ? r + 1 : null);
      }
    } catch (e) { console.error("leaderboard save", e); }
    setSessionId(null);
  };

  if (!song) {
    return (
      <>
        <div className="main-header">
          <div>
            <h2>Player</h2>
            <p>Pick a song from the Library to start singing.</p>
          </div>
        </div>
        <div className="empty">
          <div className="icon">🎤</div>
          <div>No song loaded</div>
        </div>
      </>
    );
  }

  const coverSrc = song.cover_path ? convertFileSrc(song.cover_path) : null;

  return (
    <div className="player">
      <div className="now-playing">
        <div className="cover">
          {coverSrc ? <img src={coverSrc} alt="" /> : "🎵"}
        </div>
        <div className="info">
          <h3>{song.title}</h3>
          <p>{song.artist}{song.album ? ` — ${song.album}` : ""}</p>
          <div className="row" style={{ marginTop: 12, gap: 16, flexWrap: "wrap" }}>
            <label className="row" style={{ gap: 6, color: "var(--text-secondary)", fontSize: 14 }}>
              <input
                type="checkbox"
                checked={preferInstrumental}
                onChange={(e) => setPreferInstrumental(e.target.checked)}
              />
              Instrumental
            </label>
            <label className="row" style={{ gap: 6, color: "var(--text-secondary)", fontSize: 14 }}>
              <input
                type="checkbox"
                checked={challenge}
                disabled={playing || !!sessionId}
                onChange={(e) => setChallenge(e.target.checked)}
              />
              🏆 Challenge
            </label>
            {wordTimestamps && (
              <span style={{ fontSize: 13, color: "var(--text-secondary)" }}>Word sync active</span>
            )}
          </div>
        </div>
      </div>

      <div className="lyrics-stage">
        {synced ? (
          <div className="lyrics-scroll">
            <div className="lyrics-pad" />
            {lrcLines.map((line, i) => {
              const isCurrent = i === currentIdx;
              const isPast = i < currentIdx;
              const cls = `lyric-line ${isCurrent ? "current" : isPast ? "past" : "future"}`;
              if (!isCurrent) {
                return (
                  <div key={i} className={cls}>
                    {line.text}
                  </div>
                );
              }
              const lineWords = wordsByLine?.[i]?.length > 0
                ? wordsByLine[i]
                : line.text.trim().split(/\s+/).map((w) => ({ word: w }));
              return (
                <div key={i} className={cls} ref={activeLineRef}>
                  {lineWords.map((w, j) => {
                    const state = j < wordIdx ? "past" : j === wordIdx ? "active" : "future";
                    const score = wordStatuses[j];
                    return (
                      <span
                        key={j}
                        className={`word ${state} ${score ? `score-${score}` : ""}`}
                      >
                        {w.word}
                      </span>
                    );
                  })}
                </div>
              );
            })}
            <div className="lyrics-pad" />
          </div>
        ) : song.lrc ? (
          <pre className="lyrics-plain">{song.lrc}</pre>
        ) : (
          <div className="lyric-line current">No lyrics available</div>
        )}
      </div>

      <div className="controls-bar">
        <button className="btn btn-icon play" onClick={toggle}>
          {playing ? "⏸" : "▶"}
        </button>
        <button className="btn btn-icon btn-secondary" onClick={stop}>
          ⏹
        </button>

        <div className="seek">
          <span className="seek-time">{formatTime(displayTime)}</span>
          <div
            ref={waveformRef}
            className="waveform"
            onClick={(e) => {
              const rect = e.currentTarget.getBoundingClientRect();
              const pct = (e.clientX - rect.left) / rect.width;
              seek(pct);
            }}
          >
            {bars.map((h, i) => (
              <div
                key={i}
                className="bar"
                style={{ height: `${h}%`, "--bar-pos": i / bars.length }}
              />
            ))}
          </div>
          <span className="seek-time">{formatTime(duration)}</span>
        </div>

        <div className="volume">
          <span>🔊</span>
          <input
            type="range"
            className="slider"
            min={0}
            max={1}
            step={0.01}
            value={volume}
            onChange={(e) => setVolume(parseFloat(e.target.value))}
          />
        </div>
      </div>

      {src && <audio ref={audioRef} src={src} preload="metadata" />}

      {askName && (
        <div className="modal-backdrop" onClick={() => setAskName(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h3>Challenge Mode</h3>
            <p>Enter your name to save your score.</p>
            <input
              className="input"
              value={playerName}
              onChange={(e) => setPlayerName(e.target.value)}
              placeholder="Your name"
              autoFocus
            />
            <div className="row" style={{ marginTop: 12, justifyContent: "flex-end", gap: 8 }}>
              <button className="btn btn-secondary" onClick={() => setAskName(false)}>Cancel</button>
              <button className="btn btn-primary" onClick={startChallenge} disabled={!playerName.trim()}>
                Start Singing
              </button>
            </div>
          </div>
        </div>
      )}

      {finalScore !== null && (
        <div className="modal-backdrop" onClick={() => setFinalScore(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h3>🎉 Final Score</h3>
            <div style={{ fontSize: 64, fontWeight: 700, textAlign: "center", margin: "16px 0" }}>
              {finalScore}
            </div>
            <div className="row" style={{ justifyContent: "space-around", color: "var(--text-secondary)" }}>
              <span>✓ {scoreState.hits}</span>
              <span>~ {scoreState.partials}</span>
              <span>✗ {scoreState.misses}</span>
            </div>
            {rank && <p style={{ textAlign: "center", marginTop: 12 }}>Rank #{rank} for this song</p>}
            <div className="row" style={{ marginTop: 16, justifyContent: "flex-end" }}>
              <button className="btn btn-primary" onClick={() => setFinalScore(null)}>OK</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
