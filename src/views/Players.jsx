import { useCallback, useEffect, useRef, useState } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";

const CK_GRADIENT = "linear-gradient(135deg, #FFB370 0%, #FF6B5A 40%, #F23D6D 100%)";

const PLAYER_COLORS = [
  { from: "#FF9A56", to: "#FF4F76" },
  { from: "#FF5BA8", to: "#C03BFF" },
  { from: "#4A90FF", to: "#1B4FAE" },
  { from: "#5EEAD4", to: "#0E8C8C" },
  { from: "#FFD86F", to: "#F39F37" },
  { from: "#A8E063", to: "#4F7A29" },
];

function generateId() { return "e" + Math.random().toString(36).slice(2, 9); }
function colorFor(idx) { return PLAYER_COLORS[idx % PLAYER_COLORS.length]; }

function SongCover({ song, size = 40, radius = 8 }) {
  const src = song?.cover_path ? convertFileSrc(song.cover_path) : null;
  return (
    <div style={{
      width: size, height: size, borderRadius: radius, flexShrink: 0,
      background: src
        ? `url(${src}) center/cover no-repeat`
        : "linear-gradient(135deg, #FF9A76, #FF4F76)",
      boxShadow: "0 2px 8px rgba(0,0,0,0.35)",
    }}/>
  );
}

// ─── Song Picker Modal ────────────────────────────────────────────────────────
function SongPickerModal({ songs, onSelect, onClose }) {
  const [search, setSearch] = useState("");
  const inputRef = useRef(null);
  useEffect(() => { inputRef.current?.focus(); }, []);

  const filtered = songs.filter(s => {
    const q = search.toLowerCase();
    return (s.title || "").toLowerCase().includes(q) || (s.artist || "").toLowerCase().includes(q);
  });

  return (
    <div onClick={onClose} style={{
      position: "fixed", inset: 0, zIndex: 1000,
      background: "rgba(0,0,0,0.7)", backdropFilter: "blur(8px)",
      display: "flex", alignItems: "center", justifyContent: "center",
    }}>
      <div onClick={e => e.stopPropagation()} style={{
        width: 480, maxHeight: "70vh",
        borderRadius: 18, background: "#0E0C18",
        border: "1px solid rgba(255,255,255,0.1)",
        boxShadow: "0 30px 80px rgba(0,0,0,0.7)",
        display: "flex", flexDirection: "column", overflow: "hidden",
      }}>
        <div style={{
          padding: "18px 20px 14px",
          borderBottom: "1px solid rgba(255,255,255,0.07)",
          display: "flex", alignItems: "center", gap: 12,
        }}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" style={{ flexShrink: 0, color: "rgba(237,233,255,0.4)" }}>
            <circle cx="11" cy="11" r="8" stroke="currentColor" strokeWidth="2"/>
            <path d="m21 21-4.35-4.35" stroke="currentColor" strokeWidth="2" strokeLinecap="round"/>
          </svg>
          <input ref={inputRef} value={search} onChange={e => setSearch(e.target.value)}
            placeholder="Cerca canzone o artista…"
            style={{ flex: 1, border: "none", outline: "none", background: "transparent", fontSize: 14, fontWeight: 500, color: "#FFF", fontFamily: "inherit" }}
          />
          <button onClick={onClose} style={{
            all: "unset", cursor: "pointer", width: 28, height: 28, borderRadius: 8,
            display: "flex", alignItems: "center", justifyContent: "center",
            color: "rgba(237,233,255,0.4)", background: "rgba(255,255,255,0.05)",
          }}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none"><path d="M18 6L6 18M6 6l12 12" stroke="currentColor" strokeWidth="2" strokeLinecap="round"/></svg>
          </button>
        </div>
        <div style={{ overflowY: "auto", flex: 1 }}>
          {filtered.length === 0 && (
            <div style={{ padding: "24px 20px", fontSize: 13, color: "rgba(237,233,255,0.3)", textAlign: "center" }}>
              {songs.length === 0 ? "Nessuna canzone in libreria." : "Nessun risultato."}
            </div>
          )}
          {filtered.map(s => (
            <button key={s._dir} onClick={() => onSelect(s)} style={{
              all: "unset", cursor: "pointer", boxSizing: "border-box",
              width: "100%", padding: "10px 20px",
              display: "flex", alignItems: "center", gap: 12,
              borderBottom: "1px solid rgba(255,255,255,0.04)", transition: "background 120ms",
            }}
            onMouseEnter={e => e.currentTarget.style.background = "rgba(255,255,255,0.05)"}
            onMouseLeave={e => e.currentTarget.style.background = "transparent"}
            >
              <SongCover song={s} size={42} radius={8}/>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13.5, fontWeight: 600, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {s.title || "Untitled"}
                </div>
                <div style={{ fontSize: 11.5, color: "rgba(237,233,255,0.45)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", marginTop: 2 }}>
                  {s.artist || "Unknown artist"}
                </div>
              </div>
              {s.has_instrumental && (
                <span style={{ fontSize: 9.5, fontWeight: 700, letterSpacing: 0.6, textTransform: "uppercase", padding: "2px 7px", borderRadius: 999, background: "rgba(34,211,164,0.12)", color: "#22D3A4", flexShrink: 0 }}>
                  base
                </span>
              )}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

// ─── Editable entry row ───────────────────────────────────────────────────────
function EntryRow({ entry, index, songs, totalCount, onNameChange, onSongChange, onRemove, onMove, isDragging, isOver, onDragStart, onDragOver, onDrop, onDragEnd }) {
  const [pickerOpen, setPickerOpen] = useState(false);
  const song = songs.find(s => s._dir === entry.song_dir) || null;
  const c = colorFor(index);

  return (
    <>
      <div
        draggable
        onDragStart={onDragStart} onDragOver={onDragOver} onDrop={onDrop} onDragEnd={onDragEnd}
        style={{
          display: "flex", alignItems: "center", gap: 12, padding: "10px 16px",
          background: isOver ? "rgba(255,107,90,0.08)" : isDragging ? "rgba(255,255,255,0.03)" : "transparent",
          borderTop: isOver ? "2px solid rgba(255,107,90,0.5)" : "2px solid transparent",
          opacity: isDragging ? 0.4 : 1, transition: "background 100ms", cursor: "grab",
        }}
      >
        {/* drag handle */}
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" style={{ flexShrink: 0, color: "rgba(237,233,255,0.2)" }}>
          <circle cx="9"  cy="6"  r="1.5" fill="currentColor"/>
          <circle cx="15" cy="6"  r="1.5" fill="currentColor"/>
          <circle cx="9"  cy="12" r="1.5" fill="currentColor"/>
          <circle cx="15" cy="12" r="1.5" fill="currentColor"/>
          <circle cx="9"  cy="18" r="1.5" fill="currentColor"/>
          <circle cx="15" cy="18" r="1.5" fill="currentColor"/>
        </svg>

        {/* position badge */}
        <div style={{
          width: 26, height: 26, borderRadius: "50%", flexShrink: 0,
          background: `linear-gradient(135deg, ${c.from}, ${c.to})`,
          display: "flex", alignItems: "center", justifyContent: "center",
          fontSize: 11, fontWeight: 800, color: "#FFF", fontFamily: "var(--font-display)",
          boxShadow: `0 3px 10px ${c.from}55`,
        }}>
          {index + 1}
        </div>

        {/* player name */}
        <input
          value={entry.player_name}
          onChange={e => onNameChange(e.target.value)}
          placeholder="Nome giocatore…"
          style={{
            width: 130, flexShrink: 0, border: "none", outline: "none",
            background: "rgba(255,255,255,0.05)", borderRadius: 8, padding: "6px 10px",
            fontSize: 13, fontWeight: 600, color: "#FFF", fontFamily: "inherit", transition: "background 120ms",
          }}
          onFocus={e => e.currentTarget.style.background = "rgba(255,255,255,0.09)"}
          onBlur={e => e.currentTarget.style.background = "rgba(255,255,255,0.05)"}
        />

        {/* song picker */}
        <button onClick={() => setPickerOpen(true)} style={{
          all: "unset", cursor: "pointer", flex: 1, minWidth: 0,
          display: "flex", alignItems: "center", gap: 10, padding: "6px 10px", borderRadius: 8,
          background: song ? "rgba(255,255,255,0.04)" : "rgba(255,107,90,0.08)",
          border: song ? "1px solid rgba(255,255,255,0.07)" : "1px solid rgba(255,107,90,0.25)",
          transition: "all 120ms",
        }}
        onMouseEnter={e => e.currentTarget.style.background = "rgba(255,255,255,0.08)"}
        onMouseLeave={e => e.currentTarget.style.background = song ? "rgba(255,255,255,0.04)" : "rgba(255,107,90,0.08)"}
        >
          {song ? (
            <>
              <SongCover song={song} size={28} radius={5}/>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 12.5, fontWeight: 600, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{song.title || "Untitled"}</div>
                <div style={{ fontSize: 10.5, color: "rgba(237,233,255,0.4)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{song.artist || "Unknown"}</div>
              </div>
              <svg width="11" height="11" viewBox="0 0 24 24" fill="none" style={{ flexShrink: 0, color: "rgba(237,233,255,0.3)" }}>
                <path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7" stroke="currentColor" strokeWidth="2" strokeLinecap="round"/>
                <path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"/>
              </svg>
            </>
          ) : (
            <div style={{ flex: 1, fontSize: 12.5, fontWeight: 600, color: "rgba(255,107,90,0.8)", display: "flex", alignItems: "center", gap: 8 }}>
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none">
                <path d="M9 18V5l10-2v13" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"/>
                <circle cx="6" cy="18" r="3" stroke="currentColor" strokeWidth="1.8"/>
                <circle cx="16" cy="16" r="3" stroke="currentColor" strokeWidth="1.8"/>
              </svg>
              Scegli canzone…
            </div>
          )}
        </button>

        {/* up/down */}
        <div style={{ display: "flex", flexDirection: "column", gap: 2, flexShrink: 0 }}>
          {[[-1, "↑"], [1, "↓"]].map(([dir, label]) => (
            <button key={dir} onClick={() => onMove(dir)}
              disabled={(dir === -1 && index === 0) || (dir === 1 && index === totalCount - 1)}
              style={{ all: "unset", cursor: "pointer", width: 20, height: 18, display: "flex", alignItems: "center", justifyContent: "center", fontSize: 11, color: "rgba(237,233,255,0.25)", borderRadius: 4, background: "rgba(255,255,255,0.03)", transition: "all 120ms" }}
              onMouseEnter={e => { e.currentTarget.style.color = "#FFF"; e.currentTarget.style.background = "rgba(255,255,255,0.08)"; }}
              onMouseLeave={e => { e.currentTarget.style.color = "rgba(237,233,255,0.25)"; e.currentTarget.style.background = "rgba(255,255,255,0.03)"; }}
            >{label}</button>
          ))}
        </div>

        {/* remove */}
        <button onClick={onRemove} style={{
          all: "unset", cursor: "pointer", flexShrink: 0,
          width: 26, height: 26, borderRadius: 7,
          display: "flex", alignItems: "center", justifyContent: "center",
          color: "rgba(237,233,255,0.25)", background: "rgba(255,255,255,0.03)", transition: "all 120ms",
        }}
        onMouseEnter={e => { e.currentTarget.style.color = "#F23D6D"; e.currentTarget.style.background = "rgba(242,61,109,0.12)"; }}
        onMouseLeave={e => { e.currentTarget.style.color = "rgba(237,233,255,0.25)"; e.currentTarget.style.background = "rgba(255,255,255,0.03)"; }}
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none"><path d="M18 6L6 18M6 6l12 12" stroke="currentColor" strokeWidth="2" strokeLinecap="round"/></svg>
        </button>
      </div>

      {pickerOpen && <SongPickerModal songs={songs} onSelect={s => { onSongChange(s._dir); setPickerOpen(false); }} onClose={() => setPickerOpen(false)}/>}
    </>
  );
}

// ─── Active queue (read-only with current position) ───────────────────────────
function ActiveQueuePanel({ activeQueue, queueIdx, onClearPast, onRemoveEntry }) {
  const [confirmId, setConfirmId] = useState(null);
  const hasPast = queueIdx > 0;
  return (
    <div style={{
      borderRadius: "var(--radius-lg)",
      background: "rgba(255,255,255,0.02)",
      border: "1px solid rgba(255,255,255,0.08)",
      overflow: "hidden",
      marginBottom: 24,
    }}>
      {/* Header */}
      <div style={{
        padding: "11px 16px",
        borderBottom: "1px solid rgba(255,255,255,0.06)",
        display: "flex", alignItems: "center", justifyContent: "space-between",
      }}>
        <div style={{ fontSize: 10.5, fontWeight: 700, letterSpacing: 1.2, textTransform: "uppercase", color: "rgba(237,233,255,0.4)" }}>
          Coda attiva
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          {hasPast && (
            <button onClick={onClearPast} style={{
              all: "unset", cursor: "pointer",
              fontSize: 10.5, fontWeight: 700, letterSpacing: 0.5,
              color: "rgba(237,233,255,0.4)", padding: "3px 9px", borderRadius: 6,
              background: "rgba(255,255,255,0.05)", border: "1px solid rgba(255,255,255,0.08)",
              transition: "all 120ms",
            }}
            onMouseEnter={e => { e.currentTarget.style.color = "#FF9070"; e.currentTarget.style.borderColor = "rgba(255,107,90,0.3)"; e.currentTarget.style.background = "rgba(255,107,90,0.07)"; }}
            onMouseLeave={e => { e.currentTarget.style.color = "rgba(237,233,255,0.4)"; e.currentTarget.style.borderColor = "rgba(255,255,255,0.08)"; e.currentTarget.style.background = "rgba(255,255,255,0.05)"; }}
            >
              Svuota passati
            </button>
          )}
          <div style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 11, color: "rgba(237,233,255,0.35)" }}>
            <span style={{ width: 7, height: 7, borderRadius: "50%", background: "#22D3A4", boxShadow: "0 0 8px #22D3A4aa", display: "inline-block" }}/>
            In corso
          </div>
        </div>
      </div>

      {/* Confirm remove modal */}
      {confirmId && (() => {
        const target = activeQueue.find(e => e.id === confirmId);
        return (
          <div onClick={() => setConfirmId(null)} style={{
            position: "fixed", inset: 0, zIndex: 1100,
            background: "rgba(0,0,0,0.6)", backdropFilter: "blur(6px)",
            display: "flex", alignItems: "center", justifyContent: "center",
          }}>
            <div onClick={e => e.stopPropagation()} style={{
              width: 320, borderRadius: 16,
              background: "#0E0C18", border: "1px solid rgba(255,255,255,0.1)",
              boxShadow: "0 24px 64px rgba(0,0,0,0.7)",
              padding: "24px 24px 20px",
              display: "flex", flexDirection: "column", gap: 16,
            }}>
              <div>
                <div style={{ fontSize: 15, fontWeight: 800, color: "#FFF", marginBottom: 6 }}>
                  Rimuovi dalla coda?
                </div>
                <div style={{ fontSize: 12.5, color: "rgba(237,233,255,0.5)", lineHeight: 1.5 }}>
                  <span style={{ color: "#FFF", fontWeight: 700 }}>{target?.player_name || "Questo cantante"}</span>
                  {target?.song?.title ? ` · ${target.song.title}` : ""} verrà rimosso dalla coda.
                </div>
              </div>
              <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
                <button onClick={() => setConfirmId(null)} style={{
                  all: "unset", cursor: "pointer",
                  padding: "8px 16px", borderRadius: 9,
                  fontSize: 12.5, fontWeight: 700,
                  color: "rgba(237,233,255,0.5)",
                  background: "rgba(255,255,255,0.05)",
                  border: "1px solid rgba(255,255,255,0.08)",
                  transition: "all 120ms",
                }}
                onMouseEnter={e => { e.currentTarget.style.color = "#FFF"; e.currentTarget.style.background = "rgba(255,255,255,0.09)"; }}
                onMouseLeave={e => { e.currentTarget.style.color = "rgba(237,233,255,0.5)"; e.currentTarget.style.background = "rgba(255,255,255,0.05)"; }}
                >
                  Annulla
                </button>
                <button onClick={() => { onRemoveEntry(confirmId); setConfirmId(null); }} style={{
                  all: "unset", cursor: "pointer",
                  padding: "8px 16px", borderRadius: 9,
                  fontSize: 12.5, fontWeight: 700,
                  color: "#FFF",
                  background: "rgba(242,61,109,0.8)",
                  border: "1px solid rgba(242,61,109,0.4)",
                  boxShadow: "0 4px 14px rgba(242,61,109,0.3)",
                  transition: "all 120ms",
                }}
                onMouseEnter={e => e.currentTarget.style.background = "#F23D6D"}
                onMouseLeave={e => e.currentTarget.style.background = "rgba(242,61,109,0.8)"}
                >
                  Rimuovi
                </button>
              </div>
            </div>
          </div>
        );
      })()}

      {/* Rows */}
      {activeQueue.map((entry, idx) => {
        const isCurrent = idx === queueIdx;
        const isPast    = idx < queueIdx;
        const c = colorFor(idx);
        return (
          <div key={entry.id} style={{
            display: "flex", alignItems: "center", gap: 12,
            padding: "10px 16px",
            background: isCurrent ? "rgba(255,107,90,0.07)" : "transparent",
            borderLeft: isCurrent ? "3px solid #FF6B5A" : "3px solid transparent",
            opacity: isPast ? 0.35 : 1,
            transition: "all 200ms",
          }}>
            {/* Position badge */}
            <div style={{
              width: 26, height: 26, borderRadius: "50%", flexShrink: 0,
              background: isCurrent
                ? CK_GRADIENT
                : isPast
                  ? "rgba(255,255,255,0.06)"
                  : `linear-gradient(135deg, ${c.from}, ${c.to})`,
              display: "flex", alignItems: "center", justifyContent: "center",
              fontSize: 11, fontWeight: 800, color: "#FFF", fontFamily: "var(--font-display)",
            }}>
              {isPast
                ? <svg width="10" height="10" viewBox="0 0 24 24" fill="none"><path d="M20 6L9 17l-5-5" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"/></svg>
                : isCurrent
                  ? <svg width="9" height="9" viewBox="0 0 24 24" fill="currentColor"><path d="M5 4l14 8-14 8V4z"/></svg>
                  : idx + 1
              }
            </div>

            {/* Player name */}
            <div style={{ width: 120, flexShrink: 0, fontSize: 13, fontWeight: 700, color: isCurrent ? "#FFF" : "rgba(237,233,255,0.7)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {entry.player_name || "—"}
            </div>

            {/* Song */}
            <div style={{ flex: 1, minWidth: 0, display: "flex", alignItems: "center", gap: 8 }}>
              <SongCover song={entry.song} size={28} radius={5}/>
              <div style={{ minWidth: 0 }}>
                <div style={{ fontSize: 12, fontWeight: 600, color: isCurrent ? "#FFF" : "rgba(237,233,255,0.6)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {entry.song?.title || "—"}
                </div>
                {entry.song?.artist && (
                  <div style={{ fontSize: 10.5, color: "rgba(237,233,255,0.35)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {entry.song.artist}
                  </div>
                )}
              </div>
            </div>

            {/* Status label / remove */}
            {isCurrent ? (
              <span style={{ fontSize: 10, fontWeight: 700, letterSpacing: 0.7, textTransform: "uppercase", padding: "3px 8px", borderRadius: 999, background: "rgba(255,107,90,0.15)", color: "#FF9070", flexShrink: 0 }}>
                ora
              </span>
            ) : (
              <button onClick={() => setConfirmId(entry.id)} style={{
                all: "unset", cursor: "pointer", flexShrink: 0,
                width: 22, height: 22, borderRadius: 6,
                display: "flex", alignItems: "center", justifyContent: "center",
                color: "rgba(237,233,255,0.2)", background: "transparent", transition: "all 120ms",
              }}
              onMouseEnter={e => { e.currentTarget.style.color = "#F23D6D"; e.currentTarget.style.background = "rgba(242,61,109,0.12)"; }}
              onMouseLeave={e => { e.currentTarget.style.color = "rgba(237,233,255,0.2)"; e.currentTarget.style.background = "transparent"; }}
              >
                <svg width="10" height="10" viewBox="0 0 24 24" fill="none"><path d="M18 6L6 18M6 6l12 12" stroke="currentColor" strokeWidth="2" strokeLinecap="round"/></svg>
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}

// ─── Main View ─────────────────────────────────────────────────────────────────
export default function Players({ songs = [], onStartQueue, onAddToQueue, activeQueue = [], queueIdx = -1, onClearPastFromQueue, onRemoveFromQueue }) {
  const [entries, setEntries]   = useState([]);
  const [loading, setLoading]   = useState(true);
  const [dragIdx, setDragIdx]   = useState(null);
  const [overIdx, setOverIdx]   = useState(null);
  const saveTimerRef = useRef(null);
  const queueActive = activeQueue.length > 0;

  // Load persisted queue on mount
  useEffect(() => {
    invoke("players_load")
      .then(data => setEntries(data))
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  const scheduleSave = useCallback((updated) => {
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    saveTimerRef.current = setTimeout(() => {
      invoke("players_save", { players: updated }).catch(console.error);
    }, 300);
  }, []);

  const update = useCallback((fn) => {
    setEntries(prev => {
      const next = typeof fn === "function" ? fn(prev) : fn;
      scheduleSave(next);
      return next;
    });
  }, [scheduleSave]);

  const addEntry = () => update(prev => [...prev, { id: generateId(), player_name: "", song_dir: "" }]);
  const removeEntry = (id) => update(prev => prev.filter(e => e.id !== id));
  const setName = (id, name) => update(prev => prev.map(e => e.id === id ? { ...e, player_name: name } : e));
  const setSong = (id, dir)  => update(prev => prev.map(e => e.id === id ? { ...e, song_dir: dir } : e));
  const moveEntry = (idx, dir) => {
    const to = idx + dir;
    if (to < 0 || to >= entries.length) return;
    update(prev => { const next = [...prev]; [next[idx], next[to]] = [next[to], next[idx]]; return next; });
  };

  const handleDragStart = (e, idx) => { setDragIdx(idx); e.dataTransfer.effectAllowed = "move"; };
  const handleDragOver  = (e, idx) => { e.preventDefault(); e.dataTransfer.dropEffect = "move"; setOverIdx(idx); };
  const handleDrop      = (e, toIdx) => {
    e.preventDefault();
    if (dragIdx === null || dragIdx === toIdx) { setDragIdx(null); setOverIdx(null); return; }
    update(prev => { const next = [...prev]; const [m] = next.splice(dragIdx, 1); next.splice(toIdx, 0, m); return next; });
    setDragIdx(null); setOverIdx(null);
  };
  const handleDragEnd = () => { setDragIdx(null); setOverIdx(null); };

  const readyEntries = entries.filter(e => e.player_name.trim() && e.song_dir);
  const canStart = readyEntries.length > 0 && !queueActive;

  const handleStart = () => {
    if (!canStart) return;
    const resolved = readyEntries
      .map(e => ({ ...e, song: songs.find(s => s._dir === e.song_dir) || null }))
      .filter(e => e.song !== null);
    if (resolved.length === 0) return;
    onStartQueue?.(resolved);
    // Clear local list — queue is now managed by App
    update(() => []);
  };

  // Quick-add to active queue (when queue is already running)
  const [quickName, setQuickName] = useState("");
  const [quickSongDir, setQuickSongDir] = useState("");
  const [quickPickerOpen, setQuickPickerOpen] = useState(false);
  const quickSong = songs.find(s => s._dir === quickSongDir) || null;

  const handleQuickAdd = () => {
    if (!quickName.trim() || !quickSongDir) return;
    const song = songs.find(s => s._dir === quickSongDir);
    if (!song) return;
    onAddToQueue?.({ id: generateId(), player_name: quickName.trim(), song_dir: quickSongDir, song });
    setQuickName("");
    setQuickSongDir("");
  };

  if (loading) return <div style={{ padding: "32px 36px", color: "rgba(237,233,255,0.4)", fontSize: 14 }}>Caricamento…</div>;

  return (
    <div style={{ padding: "28px 36px 40px", overflowY: "auto", height: "100%", boxSizing: "border-box" }}>
      {/* Header */}
      <div style={{ marginBottom: 28 }}>
        <div style={{ fontSize: 10.5, fontWeight: 700, letterSpacing: 1.4, textTransform: "uppercase", color: "rgba(237,233,255,0.4)", marginBottom: 6 }}>
          Scaletta serata
        </div>
        <h1 style={{ margin: 0, fontSize: 28, fontWeight: 800, fontFamily: "var(--font-display)", letterSpacing: -0.6, color: "#FFF", lineHeight: 1.1 }}>
          Song Queue
        </h1>
        <p style={{ margin: "6px 0 0", fontSize: 13, color: "rgba(237,233,255,0.5)", lineHeight: 1.5 }}>
          {queueActive
            ? "Coda in corso. Puoi aggiungere altri giocatori alla fine."
            : "Aggiungi i giocatori in ordine, assegna una canzone a ognuno e poi premi Start."}
        </p>
      </div>

      {/* ── Active queue (shown when queue is running) ── */}
      {queueActive && <ActiveQueuePanel activeQueue={activeQueue} queueIdx={queueIdx} onClearPast={onClearPastFromQueue} onRemoveEntry={onRemoveFromQueue}/>}

      {/* ── Editor / quick-add ── */}
      {queueActive ? (
        /* Quick-add row when queue is already running */
        <div>
          <div style={{ fontSize: 10.5, fontWeight: 700, letterSpacing: 1.2, textTransform: "uppercase", color: "rgba(237,233,255,0.35)", marginBottom: 10 }}>
            Aggiungi in coda
          </div>
          <div style={{
            display: "flex", alignItems: "center", gap: 10,
            padding: "12px 16px", borderRadius: "var(--radius-md)",
            background: "rgba(255,255,255,0.02)", border: "1px solid rgba(255,255,255,0.07)",
          }}>
            <input
              value={quickName}
              onChange={e => setQuickName(e.target.value)}
              placeholder="Nome giocatore…"
              onKeyDown={e => e.key === "Enter" && handleQuickAdd()}
              style={{
                width: 150, flexShrink: 0, border: "none", outline: "none",
                background: "rgba(255,255,255,0.06)", borderRadius: 8, padding: "7px 11px",
                fontSize: 13, fontWeight: 600, color: "#FFF", fontFamily: "inherit",
              }}
            />
            <button onClick={() => setQuickPickerOpen(true)} style={{
              all: "unset", cursor: "pointer", flex: 1, minWidth: 0,
              display: "flex", alignItems: "center", gap: 10, padding: "7px 11px", borderRadius: 8,
              background: quickSong ? "rgba(255,255,255,0.05)" : "rgba(255,107,90,0.07)",
              border: quickSong ? "1px solid rgba(255,255,255,0.07)" : "1px solid rgba(255,107,90,0.2)",
            }}>
              {quickSong ? (
                <>
                  <SongCover song={quickSong} size={26} radius={4}/>
                  <span style={{ fontSize: 12.5, fontWeight: 600, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {quickSong.title}
                  </span>
                </>
              ) : (
                <span style={{ fontSize: 12.5, fontWeight: 600, color: "rgba(255,107,90,0.7)" }}>Scegli canzone…</span>
              )}
            </button>
            <button
              onClick={handleQuickAdd}
              disabled={!quickName.trim() || !quickSongDir}
              style={{
                all: "unset", cursor: quickName.trim() && quickSongDir ? "pointer" : "not-allowed",
                flexShrink: 0, padding: "8px 16px", borderRadius: 9,
                background: quickName.trim() && quickSongDir ? CK_GRADIENT : "rgba(255,255,255,0.05)",
                color: quickName.trim() && quickSongDir ? "#FFF" : "rgba(237,233,255,0.25)",
                fontSize: 12.5, fontWeight: 700,
                boxShadow: quickName.trim() && quickSongDir ? "0 5px 16px rgba(242,61,109,0.3)" : "none",
                transition: "all 150ms",
              }}
            >
              + Aggiungi
            </button>
          </div>
          {quickPickerOpen && (
            <SongPickerModal songs={songs} onSelect={s => { setQuickSongDir(s._dir); setQuickPickerOpen(false); }} onClose={() => setQuickPickerOpen(false)}/>
          )}
        </div>
      ) : (
        /* Full editor when no queue is running */
        <>
          <div style={{
            borderRadius: "var(--radius-lg)", background: "rgba(255,255,255,0.02)",
            border: "1px solid rgba(255,255,255,0.07)", overflow: "hidden", marginBottom: 14,
          }}>
            {/* Column headers */}
            <div style={{
              display: "flex", alignItems: "center", gap: 12, padding: "10px 16px",
              borderBottom: "1px solid rgba(255,255,255,0.06)",
              fontSize: 10, fontWeight: 700, letterSpacing: 1.1, textTransform: "uppercase", color: "rgba(237,233,255,0.3)",
            }}>
              <div style={{ width: 14 }}/><div style={{ width: 26 }}>#</div>
              <div style={{ width: 130 }}>Giocatore</div>
              <div style={{ flex: 1 }}>Canzone</div>
              <div style={{ width: 44 }}/><div style={{ width: 26 }}/>
            </div>

            {entries.length === 0 ? (
              <div style={{ padding: "32px 20px", textAlign: "center", fontSize: 13, color: "rgba(237,233,255,0.25)", fontStyle: "italic" }}>
                Nessun giocatore — premi "Aggiungi giocatore" per iniziare.
              </div>
            ) : (
              entries.map((entry, idx) => (
                <EntryRow
                  key={entry.id} entry={entry} index={idx} songs={songs} totalCount={entries.length}
                  isDragging={dragIdx === idx} isOver={overIdx === idx && overIdx !== dragIdx}
                  onNameChange={name => setName(entry.id, name)}
                  onSongChange={dir  => setSong(entry.id, dir)}
                  onRemove={() => removeEntry(entry.id)}
                  onMove={dir => moveEntry(idx, dir)}
                  onDragStart={e => handleDragStart(e, idx)}
                  onDragOver={e  => handleDragOver(e, idx)}
                  onDrop={e      => handleDrop(e, idx)}
                  onDragEnd={handleDragEnd}
                />
              ))
            )}
          </div>

          {/* Add row */}
          <button onClick={addEntry} style={{
            all: "unset", cursor: "pointer", boxSizing: "border-box", width: "100%",
            padding: "11px 16px", borderRadius: "var(--radius-md)",
            border: "2px dashed rgba(255,255,255,0.1)", background: "transparent",
            display: "flex", alignItems: "center", gap: 10,
            fontSize: 13, fontWeight: 600, color: "rgba(237,233,255,0.35)", transition: "all 160ms",
            marginBottom: 22,
          }}
          onMouseEnter={e => { e.currentTarget.style.borderColor = "rgba(255,107,90,0.35)"; e.currentTarget.style.color = "#FF9070"; e.currentTarget.style.background = "rgba(255,107,90,0.04)"; }}
          onMouseLeave={e => { e.currentTarget.style.borderColor = "rgba(255,255,255,0.1)"; e.currentTarget.style.color = "rgba(237,233,255,0.35)"; e.currentTarget.style.background = "transparent"; }}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth="2" strokeLinecap="round"/></svg>
            Aggiungi giocatore
          </button>

          {/* Footer CTA */}
          <div style={{
            borderRadius: "var(--radius-lg)", background: "rgba(255,255,255,0.02)",
            border: "1px solid rgba(255,255,255,0.07)",
            padding: "16px 20px", display: "flex", alignItems: "center", justifyContent: "space-between", gap: 20,
          }}>
            <div style={{ fontSize: 12.5, color: "rgba(237,233,255,0.4)", lineHeight: 1.5 }}>
              {canStart
                ? <><span style={{ color: "#FFF", fontWeight: 700 }}>{readyEntries.length}</span> {readyEntries.length === 1 ? "giocatore" : "giocatori"} · l'app avanza automaticamente tra una canzone e l'altra</>
                : "Aggiungi almeno un giocatore con nome e canzone per iniziare."}
            </div>
            <button onClick={handleStart} disabled={!canStart} style={{
              all: "unset", cursor: canStart ? "pointer" : "not-allowed", flexShrink: 0,
              padding: "11px 24px", borderRadius: 11,
              background: canStart ? CK_GRADIENT : "rgba(255,255,255,0.06)",
              color: canStart ? "#FFF" : "rgba(237,233,255,0.25)",
              fontSize: 14, fontWeight: 700, letterSpacing: -0.1,
              boxShadow: canStart ? "0 8px 24px rgba(242,61,109,0.35), inset 0 1px 0 rgba(255,255,255,0.2)" : "none",
              display: "flex", alignItems: "center", gap: 8, transition: "all 160ms",
            }}>
              Add to queue
              <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"><path d="M5 4l14 8-14 8V4z"/></svg>
            </button>
          </div>
        </>
      )}
    </div>
  );
}
