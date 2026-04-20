import { useEffect, useMemo, useRef, useState, useCallback } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const CK_GRADIENT = "linear-gradient(135deg, #FFB370 0%, #FF6B5A 40%, #F23D6D 100%)";

function formatTime(sec) {
  const s = Math.max(0, Math.floor(sec || 0));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function parseLrc(text) {
  if (!text) return [];
  const re = /\[(\d{2}):(\d{2})\.(\d{2,3})\]\s*(.*)/;
  return text.split("\n").map((l) => {
    const m = l.trim().match(re);
    if (!m) return null;
    const mm = parseInt(m[1], 10), ss = parseInt(m[2], 10);
    const frac = m[3];
    const ms = frac.length === 2 ? parseInt(frac, 10) * 10 : parseInt(frac, 10);
    const t = m[4].trim();
    if (!t) return null;
    return { ts_ms: (mm * 60 + ss) * 1000 + ms, text: t };
  }).filter(Boolean).sort((a, b) => a.ts_ms - b.ts_ms);
}

function isSynced(text) { return /\[\d{2}:\d{2}\.\d{2,3}\]/.test(text || ""); }

function useBars(song, count = 72) {
  return useMemo(() => {
    const seed = (song?.title || "x").split("").reduce((acc, c) => (acc * 31 + c.charCodeAt(0)) >>> 0, 7);
    let s = seed || 1;
    const next = () => { s = (s * 1103515245 + 12345) & 0x7fffffff; return s / 0x7fffffff; };
    return Array.from({ length: count }, () => 8 + Math.abs(Math.sin(count * 0.4) + Math.cos(count * 0.12)) * 22 + next() * 10);
  }, [song, count]);
}

// Pill toggle (Instrumental / Challenge / Word sync)
function Toggle({ label, active, disabled, onChange, icon }) {
  return (
    <div
      onClick={disabled ? undefined : onChange}
      style={{
        display: "inline-flex", alignItems: "center", gap: 6,
        padding: "6px 12px", borderRadius: 999, cursor: disabled ? "default" : "pointer",
        fontSize: 11.5, fontWeight: 600,
        color: active ? "#FFF" : "rgba(237,233,255,0.5)",
        background: active ? "rgba(255,107,90,0.18)" : "rgba(255,255,255,0.04)",
        border: active ? "1px solid rgba(255,107,90,0.35)" : "1px solid rgba(255,255,255,0.06)",
        opacity: disabled ? 0.5 : 1,
        transition: "all 140ms",
      }}
    >
      <span style={{
        width: 8, height: 8, borderRadius: "50%",
        background: active ? "#FF6B5A" : "rgba(255,255,255,0.2)",
        boxShadow: active ? "0 0 8px #FF6B5A" : "none",
      }}/>
      {icon && <span style={{ fontSize: 10 }}>{icon}</span>}
      {label}
    </div>
  );
}

function AccuracyRing({ value, label, color, total = 40 }) {
  const pct = Math.min(1, value / Math.max(1, total));
  const r = 22, circ = 2 * Math.PI * r;
  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 4 }}>
      <div style={{ position: "relative", width: 56, height: 56 }}>
        <svg width="56" height="56" viewBox="0 0 56 56">
          <circle cx="28" cy="28" r={r} stroke="rgba(255,255,255,0.08)" strokeWidth="4" fill="none"/>
          <circle cx="28" cy="28" r={r} stroke={color} strokeWidth="4" fill="none"
            strokeDasharray={circ} strokeDashoffset={circ * (1 - pct)}
            strokeLinecap="round"
            style={{ transform: "rotate(-90deg)", transformOrigin: "center", transition: "stroke-dashoffset 300ms" }}/>
        </svg>
        <div style={{ position: "absolute", inset: 0, display: "flex", alignItems: "center", justifyContent: "center", fontFamily: "var(--font-mono)", fontSize: 14, fontWeight: 700, color: "#FFF" }}>
          {value}
        </div>
      </div>
      <div style={{ fontSize: 10, fontWeight: 700, letterSpacing: 1, color, textTransform: "uppercase" }}>{label}</div>
    </div>
  );
}

function Avatar({ name }) {
  const hue = [...(name || "?")].reduce((a, c) => a + c.charCodeAt(0), 0) * 37 % 360;
  return (
    <div style={{
      width: 22, height: 22, borderRadius: "50%", flexShrink: 0,
      background: `hsl(${hue}, 70%, 55%)`,
      fontSize: 9.5, fontWeight: 700, color: "#FFF",
      display: "flex", alignItems: "center", justifyContent: "center",
    }}>{(name || "?")[0]}</div>
  );
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
  const [wordStatuses, setWordStatuses] = useState({});
  const [finalScore, setFinalScore] = useState(null);
  const [rank, setRank] = useState(null);

  const [topScores, setTopScores] = useState([]);

  const lrcLines = useMemo(() => parseLrc(song?.lrc || ""), [song]);
  const synced = isSynced(song?.lrc || "");
  const bars = useBars(song);

  const wordsByLine = useMemo(() => {
    if (!wordTimestamps || lrcLines.length === 0) return null;
    const buckets = lrcLines.map(() => []);
    const hasLineField = wordTimestamps.some(w => typeof w.line === "number");
    if (hasLineField) {
      for (const w of wordTimestamps) {
        if (typeof w.line === "number" && buckets[w.line]) buckets[w.line].push(w);
      }
    } else {
      for (const w of wordTimestamps) {
        for (let i = 0; i < lrcLines.length; i++) {
          const start = lrcLines[i].ts_ms;
          const end = lrcLines[i + 1]?.ts_ms ?? (start + 8000);
          if (w.start_ms >= start && w.start_ms < end) { buckets[i].push(w); break; }
        }
      }
    }
    return buckets;
  }, [wordTimestamps, lrcLines]);

  // Line activation times — switch current line when previous line's last word ends,
  // so the next line is shown (big font, no highlight) during gaps.
  const lineActivations = useMemo(() => {
    if (lrcLines.length === 0) return [];
    if (!wordsByLine) return lrcLines.map(l => l.ts_ms);
    const result = new Array(lrcLines.length);
    let prevEnd = null;
    for (let i = 0; i < lrcLines.length; i++) {
      const ws = wordsByLine[i] || [];
      const firstStart = ws.length > 0 ? ws[0].start_ms : lrcLines[i].ts_ms;
      result[i] = prevEnd != null ? Math.min(prevEnd, firstStart) : firstStart;
      if (ws.length > 0) {
        prevEnd = ws[ws.length - 1].end_ms;
      } else {
        prevEnd = lrcLines[i + 1]?.ts_ms ?? (firstStart + 2000);
      }
    }
    return result;
  }, [wordsByLine, lrcLines]);

  useEffect(() => {
    setWordTimestamps(null);
    if (!song?._dir) return;
    invoke("get_words", { dir: song._dir })
      .then(words => { if (Array.isArray(words) && words.length > 0) setWordTimestamps(words); })
      .catch(() => {});
  }, [song]);

  useEffect(() => {
    setSrc(null);
    setDuration(song?.duration_sec || 0);
    setCurrentIdx(-1); setWordIdx(-1); setDisplayTime(0);
    setWordStatuses({}); setFinalScore(null); setRank(null);
    if (!song?._dir) return;
    (async () => {
      try {
        const path = await invoke("get_song_audio_path", { dir: song._dir, preferInstrumental });
        setSrc(convertFileSrc(path));
      } catch (e) { console.error("get_song_audio_path", e); }
    })();
  }, [song, preferInstrumental]);

  useEffect(() => {
    if (!song?._dir) return;
    invoke("leaderboard_top", { songDir: song._dir, limit: 4 })
      .then(t => setTopScores(Array.isArray(t) ? t : []))
      .catch(() => {});
  }, [song]);

  useEffect(() => { if (audioRef.current) audioRef.current.volume = volume; }, [volume]);

  const tick = useCallback(() => {
    const a = audioRef.current;
    if (!a) return;
    const t = a.currentTime, tMs = t * 1000;
    if (waveformRef.current && duration > 0) {
      waveformRef.current.style.setProperty("--progress", String(t / duration));
    }
    let newLine = -1;
    if (synced) {
      for (let i = 0; i < lrcLines.length; i++) {
        const act = lineActivations[i] ?? lrcLines[i].ts_ms;
        if (act <= tMs) newLine = i; else break;
      }
    }
    let newWord = -1;
    if (newLine >= 0) {
      if (wordsByLine && wordsByLine[newLine]?.length > 0) {
        const arr = wordsByLine[newLine];
        for (let i = 0; i < arr.length; i++) {
          if (arr[i].start_ms <= tMs) newWord = i; else break;
        }
      } else {
        const line = lrcLines[newLine];
        const words = line.text.trim().split(/\s+/);
        const lineStart = line.ts_ms;
        const lineEnd = lrcLines[newLine + 1]?.ts_ms ?? (lineStart + 3000);
        const elapsed = tMs - lineStart;
        const dur = Math.max(1, lineEnd - lineStart);
        newWord = Math.min(Math.max(Math.floor((elapsed / dur) * words.length), 0), words.length - 1);
      }
    }
    setCurrentIdx(prev => prev !== newLine ? newLine : prev);
    setWordIdx(prev => prev !== newWord ? newWord : prev);
    if (Math.abs(t - lastSeekRef.current) > 0.25) {
      lastSeekRef.current = t;
      setDisplayTime(t);
    }
    rafRef.current = requestAnimationFrame(tick);
  }, [synced, lrcLines, wordsByLine, lineActivations, duration]);

  useEffect(() => {
    if (playing) { rafRef.current = requestAnimationFrame(tick); return () => cancelAnimationFrame(rafRef.current); }
    cancelAnimationFrame(rafRef.current);
  }, [playing, tick]);

  useEffect(() => {
    const a = audioRef.current;
    if (!a) return;
    const onDur = () => setDuration(a.duration || song?.duration_sec || 0);
    const onEnd = () => { setPlaying(false); if (challenge && sessionId) endChallenge(); };
    a.addEventListener("loadedmetadata", onDur);
    a.addEventListener("ended", onEnd);
    return () => { a.removeEventListener("loadedmetadata", onDur); a.removeEventListener("ended", onEnd); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [src, song, challenge, sessionId]);

  useEffect(() => {
    if (!activeLineRef.current) return;
    activeLineRef.current.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [currentIdx]);

  useEffect(() => {
    if (!challenge) return;
    let unlisten;
    (async () => {
      unlisten = await listen("karaoke://score-tick", (ev) => {
        const { word_idx, status } = ev.payload || {};
        if (typeof word_idx !== "number") return;
        setWordStatuses(prev => ({ ...prev, [word_idx]: status }));
        setScoreState(prev => {
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
    if (playing) { a.pause(); setPlaying(false); return; }
    if (challenge && !sessionId) { setAskName(true); return; }
    try { await a.play(); setPlaying(true); } catch (e) { console.error(e); }
  };

  const stop = () => {
    const a = audioRef.current;
    if (!a) return;
    a.pause(); a.currentTime = 0; setPlaying(false);
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
      setSessionId(null); return;
    }
    setAskName(false);
    try { await audioRef.current?.play(); setPlaying(true); } catch (e) { console.error(e); }
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
          song_dir: song._dir, song_title: song.title || "",
          player_name: playerName.trim() || "Anonymous",
          score, hits: scoreState.hits, partials: scoreState.partials, misses: scoreState.misses,
        },
      });
      const top = await invoke("leaderboard_top", { songDir: song._dir, limit: 10 });
      if (Array.isArray(top)) {
        const r = top.findIndex(e => e.score === score && e.player_name === (playerName.trim() || "Anonymous"));
        setRank(r >= 0 ? r + 1 : null);
        setTopScores(top.slice(0, 4));
      }
    } catch (e) { console.error("leaderboard save", e); }
    setSessionId(null);
  };

  if (!song) {
    return (
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", height: "100%", gap: 16, color: "rgba(237,233,255,0.4)" }}>
        <div style={{ fontSize: 64, opacity: 0.4 }}>🎤</div>
        <div style={{ fontFamily: "var(--font-display)", fontSize: 20, fontWeight: 700, color: "#FFF" }}>No song loaded</div>
        <div style={{ fontSize: 14 }}>Pick a song from the Library to start singing.</div>
      </div>
    );
  }

  const coverSrc = song.cover_path ? convertFileSrc(song.cover_path) : null;
  const sessionScore = scoreState.hits + scoreState.partials + scoreState.misses === 0 ? null
    : Math.round(((scoreState.hits + 0.5 * scoreState.partials) / (scoreState.hits + scoreState.partials + scoreState.misses)) * 100);

  return (
    <div style={{ padding: "20px 28px 20px", display: "flex", flexDirection: "column", gap: 14, height: "100%", boxSizing: "border-box", minHeight: 0 }}>
      {/* Track header */}
      <div style={{
        display: "flex", gap: 16, alignItems: "center",
        padding: 14, borderRadius: 20, flexShrink: 0,
        background: "linear-gradient(105deg, rgba(255,107,90,0.08), rgba(255,255,255,0.02))",
        border: "1px solid rgba(255,255,255,0.06)",
      }}>
        <div style={{
          width: 80, height: 80, borderRadius: 12, flexShrink: 0,
          background: coverSrc ? `url(${coverSrc}) center/cover no-repeat` : "linear-gradient(135deg, #FF9A76, #FF4F76)",
          boxShadow: "0 8px 20px rgba(0,0,0,0.4)",
          overflow: "hidden",
        }}/>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 1.6, color: "#FF9070", textTransform: "uppercase", marginBottom: 4 }}>
            Now singing
          </div>
          <h2 style={{ margin: 0, fontFamily: "var(--font-display)", fontWeight: 700, fontSize: 24, letterSpacing: -0.8, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {song.title}
          </h2>
          <div style={{ marginTop: 3, fontSize: 13, color: "rgba(237,233,255,0.6)" }}>
            {song.artist}{song.album ? <span style={{ color: "rgba(237,233,255,0.4)" }}> — {song.album}</span> : null}
          </div>
          <div style={{ display: "flex", gap: 8, marginTop: 10, flexWrap: "wrap" }}>
            <Toggle label="Instrumental" active={preferInstrumental} onChange={() => setPreferInstrumental(v => !v)}/>
            <Toggle label="Challenge" active={challenge} icon="🏆" disabled={playing || !!sessionId} onChange={() => setChallenge(v => !v)}/>
            {wordTimestamps && <Toggle label="Word sync" active={true} icon="◉"/>}
          </div>
        </div>
        {/* Score chip */}
        {challenge && (
          <div style={{
            padding: "12px 16px", borderRadius: 14, flexShrink: 0,
            background: CK_GRADIENT, color: "#FFF", textAlign: "right", minWidth: 120,
            boxShadow: "0 10px 24px rgba(242,61,109,0.35), inset 0 1px 0 rgba(255,255,255,0.25)",
          }}>
            <div style={{ fontSize: 10, fontWeight: 700, letterSpacing: 1.4, textTransform: "uppercase", opacity: 0.9 }}>Score</div>
            <div style={{ fontFamily: "var(--font-display)", fontSize: 28, fontWeight: 700, letterSpacing: -1, lineHeight: 1 }}>
              {sessionScore ?? "–"}
            </div>
            <div style={{ fontSize: 11, marginTop: 4, opacity: 0.9 }}>
              {scoreState.hits}h {scoreState.partials}p {scoreState.misses}m
            </div>
          </div>
        )}
      </div>

      {/* Stage + side panel */}
      <div style={{ flex: 1, display: "grid", gridTemplateColumns: "1fr 260px", gap: 14, minHeight: 0 }}>
        {/* Lyric stage */}
        <div style={{
          position: "relative", borderRadius: 20, overflow: "hidden",
          background: "radial-gradient(ellipse at 50% 30%, rgba(255,107,90,0.1), rgba(7,6,12,0) 55%), linear-gradient(180deg, #0D0B18 0%, #07060C 100%)",
          border: "1px solid rgba(255,255,255,0.06)",
          display: "flex", flexDirection: "column", minHeight: 0,
        }}>
          {/* Stage glow blobs */}
          <div style={{ position: "absolute", top: -80, left: -40, width: 260, height: 260, background: "radial-gradient(circle, rgba(255,179,112,0.15), transparent 70%)", filter: "blur(20px)", animation: "floaty 6s ease-in-out infinite", pointerEvents: "none" }}/>
          <div style={{ position: "absolute", top: -60, right: -40, width: 280, height: 280, background: "radial-gradient(circle, rgba(242,61,109,0.15), transparent 70%)", filter: "blur(20px)", animation: "floaty 8s ease-in-out infinite reverse", pointerEvents: "none" }}/>

          {synced ? (
            <div style={{ flex: 1, overflowY: "auto", padding: "0 40px", scrollBehavior: "smooth", scrollbarWidth: "none" }}
              className="lyrics-scroll">
              <div style={{ height: "40%", flexShrink: 0 }}/>
              {lrcLines.map((line, i) => {
                const isCurrent = i === currentIdx;
                const isPast = i < currentIdx;
                const lineWords = isCurrent
                  ? (wordsByLine?.[i]?.length > 0
                    ? wordsByLine[i]
                    : line.text.trim().split(/\s+/).map(w => ({ word: w })))
                  : null;
                return (
                  <div key={i} ref={isCurrent ? activeLineRef : null} style={{
                    textAlign: "center", padding: "10px 0",
                    fontFamily: "var(--font-display)",
                    fontSize: isCurrent ? 44 : (i === currentIdx + 1 || i === currentIdx - 1) ? 22 : 18,
                    fontWeight: isCurrent ? 700 : 500,
                    letterSpacing: isCurrent ? -1.5 : -0.4,
                    lineHeight: 1.15,
                    color: isPast ? "rgba(237,233,255,0.25)" : isCurrent ? "#FFF" : (i === currentIdx + 1 ? "rgba(237,233,255,0.35)" : "rgba(237,233,255,0.2)"),
                    transition: "all 200ms ease",
                    position: "relative",
                  }}>
                    {isCurrent && lineWords ? (
                      <>
                        {lineWords.map((w, j) => {
                          const lit = j < wordIdx;
                          const active = j === wordIdx;
                          const scoreStatus = wordStatuses[j];
                          return (
                            <span key={j} style={{
                              display: "inline-block", marginRight: "0.32em",
                              color: lit ? "#FFF" : active ? "#FF9070" : "rgba(237,233,255,0.35)",
                              textShadow: lit ? "0 0 24px rgba(255,255,255,0.3)" : active ? "0 0 18px rgba(255,144,112,0.6)" : "none",
                              transition: "color 200ms ease, text-shadow 200ms ease",
                              transform: active ? "translateY(-2px)" : "none",
                              borderBottom: scoreStatus === "hit" ? "2px solid #22D3A4" : scoreStatus === "partial" ? "2px solid #FFB370" : scoreStatus === "miss" ? "2px solid #F23D6D" : "2px solid transparent",
                            }}>{w.word}</span>
                          );
                        })}
                      </>
                    ) : line.text}
                  </div>
                );
              })}
              <div style={{ height: "40%", flexShrink: 0 }}/>
            </div>
          ) : song?.lrc ? (
            <pre style={{
              flex: 1, margin: 0, padding: "20px 32px",
              color: "rgba(237,233,255,0.6)", fontSize: 14, lineHeight: 1.6,
              fontFamily: "var(--font-sans)", overflowY: "auto", whiteSpace: "pre-wrap",
              scrollbarWidth: "none",
            }}>{song.lrc}</pre>
          ) : (
            <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center", color: "rgba(237,233,255,0.3)", fontSize: 18, fontFamily: "var(--font-display)" }}>
              No lyrics available
            </div>
          )}
        </div>

        {/* Side panel */}
        <div style={{ display: "flex", flexDirection: "column", gap: 12, minHeight: 0 }}>
          {/* Accuracy rings */}
          <div style={{ padding: 16, borderRadius: 16, background: "rgba(255,255,255,0.03)", border: "1px solid rgba(255,255,255,0.06)", flexShrink: 0 }}>
            <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 1.4, color: "rgba(237,233,255,0.5)", textTransform: "uppercase", marginBottom: 12 }}>
              Accuracy
            </div>
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-around" }}>
              <AccuracyRing value={scoreState.hits} label="Hit" color="#22D3A4" total={Math.max(scoreState.hits + scoreState.partials + scoreState.misses, 40)}/>
              <AccuracyRing value={scoreState.partials} label="Par" color="#FFB370" total={Math.max(scoreState.hits + scoreState.partials + scoreState.misses, 40)}/>
              <AccuracyRing value={scoreState.misses} label="Miss" color="#F23D6D" total={Math.max(scoreState.hits + scoreState.partials + scoreState.misses, 40)}/>
            </div>
          </div>

          {/* Score / session info */}
          {challenge && (
            <div style={{
              padding: 16, borderRadius: 16, flexShrink: 0,
              background: "linear-gradient(135deg, rgba(255,107,90,0.18), rgba(242,61,109,0.08))",
              border: "1px solid rgba(255,107,90,0.25)",
            }}>
              <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 1.4, color: "#FF9070", textTransform: "uppercase", marginBottom: 8 }}>
                Session
              </div>
              <div style={{ fontFamily: "var(--font-display)", fontSize: 36, fontWeight: 700, letterSpacing: -1.5, color: "#FFF", lineHeight: 1 }}>
                {sessionScore !== null ? `${sessionScore}` : "–"}
              </div>
              <div style={{ marginTop: 4, fontSize: 11, color: "rgba(237,233,255,0.6)" }}>
                {sessionId ? "Recording…" : (challenge ? "Press play to start" : "Enable challenge mode")}
              </div>
            </div>
          )}

          {/* Leaderboard */}
          <div style={{
            padding: 16, borderRadius: 16, flex: 1, minHeight: 0,
            background: "rgba(255,255,255,0.03)", border: "1px solid rgba(255,255,255,0.06)",
            display: "flex", flexDirection: "column", overflow: "hidden",
          }}>
            <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 1.4, color: "rgba(237,233,255,0.5)", textTransform: "uppercase", marginBottom: 10 }}>
              Top scores
            </div>
            {topScores.length === 0 ? (
              <div style={{ color: "rgba(237,233,255,0.3)", fontSize: 12, textAlign: "center", marginTop: 16 }}>No scores yet</div>
            ) : (
              <div style={{ display: "flex", flexDirection: "column", gap: 6, overflowY: "auto" }}>
                {topScores.map((e, i) => (
                  <div key={e.id || i} style={{
                    display: "flex", alignItems: "center", gap: 8,
                    padding: "7px 10px", borderRadius: 10,
                    background: e.player_name === playerName ? "rgba(255,107,90,0.12)" : "transparent",
                    border: e.player_name === playerName ? "1px solid rgba(255,107,90,0.25)" : "1px solid transparent",
                  }}>
                    <div style={{ fontSize: 11, fontWeight: 700, color: i === 0 ? "#FFD166" : "rgba(237,233,255,0.5)", fontFamily: "var(--font-mono)", width: 14 }}>
                      {i + 1}
                    </div>
                    <Avatar name={e.player_name}/>
                    <div style={{ flex: 1, fontSize: 12, fontWeight: 600, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {e.player_name}
                    </div>
                    <div style={{ fontFamily: "var(--font-mono)", fontSize: 12, color: "#FFF", fontWeight: 600 }}>
                      {e.score}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Transport */}
      <div style={{
        padding: "12px 16px", borderRadius: 16, flexShrink: 0,
        background: "rgba(255,255,255,0.03)",
        border: "1px solid rgba(255,255,255,0.06)",
        display: "flex", alignItems: "center", gap: 14,
      }}>
        {/* Play */}
        <button onClick={toggle} style={{
          all: "unset", cursor: "pointer", flexShrink: 0,
          width: 46, height: 46, borderRadius: "50%",
          background: CK_GRADIENT, color: "#FFF",
          display: "flex", alignItems: "center", justifyContent: "center",
          boxShadow: "0 8px 20px rgba(242,61,109,0.4), inset 0 1px 0 rgba(255,255,255,0.25)",
        }}>
          {playing
            ? <svg width="16" height="16" viewBox="0 0 24 24"><rect x="6" y="5" width="4" height="14" fill="#FFF" rx="1"/><rect x="14" y="5" width="4" height="14" fill="#FFF" rx="1"/></svg>
            : <svg width="16" height="16" viewBox="0 0 24 24"><path d="M7 4.5v15L20 12 7 4.5z" fill="#FFF"/></svg>
          }
        </button>
        {/* Stop */}
        <button onClick={stop} style={{
          all: "unset", cursor: "pointer", flexShrink: 0,
          width: 34, height: 34, borderRadius: 10,
          display: "flex", alignItems: "center", justifyContent: "center",
          background: "rgba(255,255,255,0.05)", color: "#EDE9FF",
        }}>
          <svg width="12" height="12" viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" fill="currentColor" rx="1"/></svg>
        </button>

        <span style={{ fontSize: 11, fontFamily: "var(--font-mono)", color: "rgba(237,233,255,0.6)", minWidth: 34, flexShrink: 0 }}>
          {formatTime(displayTime)}
        </span>

        {/* Waveform scrubber */}
        <div
          ref={waveformRef}
          className="waveform"
          onClick={e => {
            const rect = e.currentTarget.getBoundingClientRect();
            seek((e.clientX - rect.left) / rect.width);
          }}
          style={{ flex: 1, height: 36, display: "flex", alignItems: "center", gap: 2, cursor: "pointer", position: "relative", "--progress": 0 }}
        >
          {bars.map((h, i) => (
            <div key={i} className="bar" style={{ height: `${h}%` }}/>
          ))}
          <div className="waveform-played" aria-hidden="true">
            {bars.map((h, i) => (
              <div key={i} className="bar" style={{ height: `${h}%` }}/>
            ))}
          </div>
        </div>

        <span style={{ fontSize: 11, fontFamily: "var(--font-mono)", color: "rgba(237,233,255,0.6)", minWidth: 34, textAlign: "right", flexShrink: 0 }}>
          {formatTime(duration)}
        </span>

        {/* Volume */}
        <div style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 110, flexShrink: 0 }}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
            <path d="M11 5L6 9H3v6h3l5 4V5z" stroke="rgba(237,233,255,0.6)" strokeWidth="1.8" strokeLinejoin="round"/>
            <path d="M15.5 9a4 4 0 010 6" stroke="rgba(237,233,255,0.6)" strokeWidth="1.8" strokeLinecap="round"/>
          </svg>
          <input type="range" className="slider" min={0} max={1} step={0.01} value={volume}
            onChange={e => setVolume(parseFloat(e.target.value))}
            style={{ flex: 1 }}
          />
        </div>

        {/* Mic status */}
        {sessionId && (
          <div style={{
            display: "flex", alignItems: "center", gap: 6, flexShrink: 0,
            padding: "5px 10px", borderRadius: 999,
            background: "rgba(34,211,164,0.12)", border: "1px solid rgba(34,211,164,0.3)",
            fontSize: 10.5, fontWeight: 700, color: "#22D3A4", letterSpacing: 0.5, textTransform: "uppercase",
          }}>
            <span style={{ width: 6, height: 6, borderRadius: "50%", background: "#22D3A4", boxShadow: "0 0 8px #22D3A4", animation: "glowPulse 1.2s ease-in-out infinite" }}/>
            Mic on
          </div>
        )}
      </div>

      {src && <audio ref={audioRef} src={src} preload="metadata"/>}

      {/* Ask name modal */}
      {askName && (
        <div style={{
          position: "fixed", inset: 0, background: "rgba(0,0,0,0.7)", backdropFilter: "blur(8px)",
          display: "flex", alignItems: "center", justifyContent: "center", zIndex: 1000,
        }} onClick={() => setAskName(false)}>
          <div style={{
            background: "#0D0B18", borderRadius: 20, padding: 28, minWidth: 360, maxWidth: 480,
            border: "1px solid rgba(255,255,255,0.08)",
            boxShadow: "0 20px 60px rgba(0,0,0,0.6)",
          }} onClick={e => e.stopPropagation()}>
            <h3 style={{ margin: "0 0 8px", fontFamily: "var(--font-display)", fontSize: 20, color: "#FFF" }}>Challenge Mode 🏆</h3>
            <p style={{ margin: "0 0 20px", color: "rgba(237,233,255,0.6)", fontSize: 13 }}>Enter your name to save your score.</p>
            <input
              style={{
                width: "100%", padding: "12px 16px", borderRadius: 12, boxSizing: "border-box",
                background: "rgba(255,255,255,0.05)", border: "1px solid rgba(255,255,255,0.1)",
                color: "#FFF", fontSize: 14, fontFamily: "var(--font-sans)", outline: "none",
              }}
              value={playerName} onChange={e => setPlayerName(e.target.value)}
              placeholder="Your name" autoFocus
            />
            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 16 }}>
              <button style={{ all: "unset", cursor: "pointer", padding: "10px 16px", borderRadius: 10, fontSize: 13, fontWeight: 600, background: "rgba(255,255,255,0.05)", color: "#EDE9FF", border: "1px solid rgba(255,255,255,0.08)" }} onClick={() => setAskName(false)}>Cancel</button>
              <button style={{ all: "unset", cursor: playerName.trim() ? "pointer" : "default", padding: "10px 16px", borderRadius: 10, fontSize: 13, fontWeight: 600, background: CK_GRADIENT, color: "#FFF", opacity: playerName.trim() ? 1 : 0.5 }} onClick={startChallenge} disabled={!playerName.trim()}>Start Singing</button>
            </div>
          </div>
        </div>
      )}

      {/* Final score modal */}
      {finalScore !== null && (
        <div style={{
          position: "fixed", inset: 0, background: "rgba(0,0,0,0.7)", backdropFilter: "blur(8px)",
          display: "flex", alignItems: "center", justifyContent: "center", zIndex: 1000,
        }} onClick={() => setFinalScore(null)}>
          <div style={{
            background: "#0D0B18", borderRadius: 20, padding: 32, minWidth: 360, textAlign: "center",
            border: "1px solid rgba(255,255,255,0.08)",
            boxShadow: "0 20px 60px rgba(0,0,0,0.6)",
          }} onClick={e => e.stopPropagation()}>
            <div style={{ fontSize: 32 }}>🎉</div>
            <h3 style={{ margin: "12px 0 4px", fontFamily: "var(--font-display)", fontSize: 20, color: "#FFF" }}>Final Score</h3>
            <div style={{ fontFamily: "var(--font-display)", fontSize: 72, fontWeight: 700, letterSpacing: -2, color: "#FFF", lineHeight: 1, margin: "16px 0" }}>
              {finalScore}
            </div>
            <div style={{ display: "flex", justifyContent: "space-around", marginBottom: 16 }}>
              <div style={{ textAlign: "center" }}>
                <div style={{ fontFamily: "var(--font-mono)", fontSize: 20, fontWeight: 700, color: "#22D3A4" }}>{scoreState.hits}</div>
                <div style={{ fontSize: 11, color: "rgba(237,233,255,0.5)", marginTop: 2 }}>HIT</div>
              </div>
              <div style={{ textAlign: "center" }}>
                <div style={{ fontFamily: "var(--font-mono)", fontSize: 20, fontWeight: 700, color: "#FFB370" }}>{scoreState.partials}</div>
                <div style={{ fontSize: 11, color: "rgba(237,233,255,0.5)", marginTop: 2 }}>PAR</div>
              </div>
              <div style={{ textAlign: "center" }}>
                <div style={{ fontFamily: "var(--font-mono)", fontSize: 20, fontWeight: 700, color: "#F23D6D" }}>{scoreState.misses}</div>
                <div style={{ fontSize: 11, color: "rgba(237,233,255,0.5)", marginTop: 2 }}>MISS</div>
              </div>
            </div>
            {rank && <p style={{ color: "rgba(237,233,255,0.6)", fontSize: 13, margin: "0 0 20px" }}>Rank #{rank} for this song</p>}
            <button style={{ all: "unset", cursor: "pointer", padding: "12px 28px", borderRadius: 12, fontSize: 14, fontWeight: 600, background: CK_GRADIENT, color: "#FFF", boxShadow: "0 8px 20px rgba(242,61,109,0.35)" }} onClick={() => setFinalScore(null)}>OK</button>
          </div>
        </div>
      )}
    </div>
  );
}
