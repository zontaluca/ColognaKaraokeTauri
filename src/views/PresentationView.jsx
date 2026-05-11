import { useEffect, useRef, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

export default function PresentationView() {
  const [song, setSong] = useState(null);
  const [lrcLines, setLrcLines] = useState([]);
  const [wordsByLine, setWordsByLine] = useState(null);
  const [currentIdx, setCurrentIdx] = useState(-1);
  const [wordIdx, setWordIdx] = useState(-1);
  const [wordStatuses, setWordStatuses] = useState({});
  const activeLineRef = useRef(null);

  useEffect(() => {
    let unInit, unTick;
    (async () => {
      unInit = await listen("karaoke://presentation-init", (ev) => {
        const p = ev.payload || {};
        if (p.song) setSong(p.song);
        if (Array.isArray(p.lrcLines)) setLrcLines(p.lrcLines);
        setWordsByLine(p.wordsByLine ?? null);
      });
      unTick = await listen("karaoke://presentation-tick", (ev) => {
        const p = ev.payload || {};
        if (typeof p.currentIdx === "number") setCurrentIdx(p.currentIdx);
        if (typeof p.wordIdx === "number") setWordIdx(p.wordIdx);
        if (p.wordStatuses && typeof p.wordStatuses === "object") setWordStatuses(p.wordStatuses);
      });
      emit("karaoke://presentation-ready", {}).catch(() => {});
    })();
    return () => { if (unInit) unInit(); if (unTick) unTick(); };
  }, []);

  useEffect(() => {
    if (!activeLineRef.current) return;
    activeLineRef.current.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [currentIdx]);

  const handleClose = () => {
    emit("karaoke://presentation-closed", {}).catch(() => {});
    invoke("close_presentation_window").catch(() => {});
  };

  const hasSynced = lrcLines.length > 0;

  return (
    <div style={{
      position: "fixed", inset: 0, background: "#000",
      color: "#FFF", fontFamily: "var(--font-display)",
      display: "flex", flexDirection: "column",
      overflow: "hidden",
    }}>
      {song && (
        <div style={{
          position: "absolute", top: "3vmin", left: "5vmin",
          fontSize: "clamp(14px, 2vmin, 36px)",
          color: "rgba(237,233,255,0.45)",
          fontWeight: 600, letterSpacing: "0.02em",
          pointerEvents: "none",
        }}>
          {song.title}{song.artist ? ` — ${song.artist}` : ""}
        </div>
      )}

      {hasSynced ? (
        <div style={{
          flex: 1, overflowY: "auto", scrollBehavior: "smooth",
          scrollbarWidth: "none", padding: "0 6vmin",
          display: "flex", flexDirection: "column",
        }}>
          <div style={{ height: "40%", flexShrink: 0 }}/>
          {lrcLines.map((line, i) => {
            const isCurrent = i === currentIdx;
            const isPast = i < currentIdx;
            const isAdjacent = i === currentIdx + 1 || i === currentIdx - 1;
            const lineWords = (isCurrent && wordsByLine?.[i]?.length > 0)
              ? wordsByLine[i]
              : null;
            return (
              <div key={i} ref={isCurrent ? activeLineRef : null} style={{
                textAlign: "center", padding: "1.2vmin 0",
                fontSize: isCurrent
                  ? "clamp(48px, 10vmin, 220px)"
                  : isAdjacent
                    ? "clamp(24px, 4.8vmin, 96px)"
                    : "clamp(18px, 3.2vmin, 64px)",
                fontWeight: isCurrent ? 700 : 500,
                letterSpacing: isCurrent ? "-0.035em" : "-0.02em",
                lineHeight: 1.15,
                color: isPast
                  ? "rgba(237,233,255,0.2)"
                  : isCurrent
                    ? "#FFF"
                    : (i === currentIdx + 1 ? "rgba(237,233,255,0.4)" : "rgba(237,233,255,0.18)"),
                transition: "all 200ms ease",
              }}>
                {isCurrent && lineWords ? lineWords.map((w, j) => {
                  const lit = j < wordIdx;
                  const active = j === wordIdx;
                  const status = wordStatuses?.[j];
                  return (
                    <span key={j} style={{
                      display: "inline-block", marginRight: "0.32em",
                      color: lit ? "#FFF" : active ? "#FF9070" : "rgba(237,233,255,0.35)",
                      textShadow: lit ? "0 0 24px rgba(255,255,255,0.3)" : active ? "0 0 18px rgba(255,144,112,0.6)" : "none",
                      transition: "color 200ms ease, text-shadow 200ms ease",
                      transform: active ? "translateY(-0.3vmin)" : "none",
                      borderBottom: status === "hit" ? "0.4vmin solid #22D3A4" : status === "partial" ? "0.4vmin solid #FFB370" : status === "miss" ? "0.4vmin solid #F23D6D" : "0.4vmin solid transparent",
                    }}>{w.word}</span>
                  );
                }) : line.text}
              </div>
            );
          })}
          <div style={{ height: "40%", flexShrink: 0 }}/>
        </div>
      ) : (
        <div style={{
          flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
          color: "rgba(237,233,255,0.35)", fontSize: "clamp(20px, 3vmin, 56px)",
        }}>
          {song ? "No synced lyrics" : "Waiting for player..."}
        </div>
      )}
      <div style={{
        position: "absolute", bottom: "2vmin", right: "2vmin",
        zIndex: 100,
      }}>
        <button onClick={handleClose} style={{
          background: "rgba(255,255,255,0.08)",
          border: "1px solid rgba(255,255,255,0.18)",
          color: "rgba(237,233,255,0.6)",
          fontSize: "clamp(11px, 1.4vmin, 18px)",
          padding: "0.6em 1.2em",
          borderRadius: "0.6em",
          cursor: "pointer",
          fontFamily: "var(--font-display)",
          letterSpacing: "0.04em",
          transition: "background 150ms ease, color 150ms ease",
        }}
          onMouseEnter={e => { e.currentTarget.style.background = "rgba(255,255,255,0.16)"; e.currentTarget.style.color = "#FFF"; }}
          onMouseLeave={e => { e.currentTarget.style.background = "rgba(255,255,255,0.08)"; e.currentTarget.style.color = "rgba(237,233,255,0.6)"; }}
        >
          Chiudi finestra
        </button>
      </div>
    </div>
  );
}
