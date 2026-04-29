import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

const CK_GRADIENT = "linear-gradient(135deg, #FFB370 0%, #FF6B5A 40%, #F23D6D 100%)";

function avatarHue(name) {
  return [...(name || "?")].reduce((a, c) => a + c.charCodeAt(0), 0) * 37 % 360;
}

function PhotoBubble({ player, size = 84 }) {
  const photo = player?.photo_path ? convertFileSrc(player.photo_path) : null;
  const initial = (player?.name || "?")[0].toUpperCase();
  const hue = avatarHue(player?.name);
  return (
    <div style={{
      width: size, height: size, borderRadius: "50%", flexShrink: 0,
      background: photo ? `url(${photo}) center/cover no-repeat` : `hsl(${hue}, 70%, 55%)`,
      display: "flex", alignItems: "center", justifyContent: "center",
      fontSize: size * 0.36, fontWeight: 700, color: "#FFF",
      border: "2px solid rgba(255,255,255,0.12)",
      boxShadow: `0 8px 24px hsla(${hue}, 70%, 55%, 0.35)`,
    }}>
      {!photo && initial}
    </div>
  );
}

async function pickImage() {
  return await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Image", extensions: ["jpg", "jpeg", "png", "webp", "gif", "heic"] }],
  });
}

function EditorModal({ player, onClose, onSaved }) {
  const isNew = !player;
  const [name, setName] = useState(player?.name || "");
  const [photoSrc, setPhotoSrc] = useState(player?.photo_path || null);
  const [pendingPath, setPendingPath] = useState(null);
  const [clearPhoto, setClearPhoto] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState(null);

  const previewSrc = useMemo(() => {
    if (clearPhoto) return null;
    if (pendingPath) return convertFileSrc(pendingPath);
    if (photoSrc) return convertFileSrc(photoSrc);
    return null;
  }, [pendingPath, photoSrc, clearPhoto]);

  const choosePhoto = async () => {
    try {
      const sel = await pickImage();
      if (typeof sel === "string" && sel) {
        setPendingPath(sel);
        setClearPhoto(false);
      }
    } catch (e) { console.error(e); }
  };

  const save = async () => {
    if (!name.trim()) { setErr("Inserisci un nome"); return; }
    setBusy(true); setErr(null);
    try {
      if (isNew) {
        const created = await invoke("players_create", { name: name.trim(), photoPath: pendingPath || null });
        onSaved(created);
      } else {
        const updated = await invoke("players_update", {
          id: player.id,
          name: name.trim() !== player.name ? name.trim() : null,
          photoPath: pendingPath || null,
          clearPhoto: clearPhoto && !pendingPath,
        });
        onSaved(updated);
      }
    } catch (e) {
      console.error(e);
      setErr(String(e));
    } finally { setBusy(false); }
  };

  const initial = (name || "?")[0].toUpperCase();
  const hue = avatarHue(name);

  return (
    <div style={{
      position: "fixed", inset: 0, background: "rgba(0,0,0,0.7)", backdropFilter: "blur(8px)",
      display: "flex", alignItems: "center", justifyContent: "center", zIndex: 1000,
    }} onClick={onClose}>
      <div style={{
        background: "#0D0B18", borderRadius: 22, padding: 28, minWidth: 380, maxWidth: 460,
        border: "1px solid rgba(255,255,255,0.08)",
        boxShadow: "0 20px 60px rgba(0,0,0,0.6)",
      }} onClick={e => e.stopPropagation()}>
        <h3 style={{ margin: "0 0 18px", fontFamily: "var(--font-display)", fontSize: 22, color: "#FFF" }}>
          {isNew ? "Nuovo profilo" : "Modifica profilo"}
        </h3>
        <div style={{ display: "flex", gap: 18, alignItems: "center", marginBottom: 18 }}>
          <div style={{
            width: 96, height: 96, borderRadius: "50%", flexShrink: 0,
            background: previewSrc ? `url(${previewSrc}) center/cover no-repeat` : `hsl(${hue}, 70%, 55%)`,
            display: "flex", alignItems: "center", justifyContent: "center",
            fontSize: 36, fontWeight: 700, color: "#FFF",
            border: "2px solid rgba(255,255,255,0.12)",
            boxShadow: `0 8px 24px hsla(${hue}, 70%, 55%, 0.35)`,
          }}>
            {!previewSrc && initial}
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            <button onClick={choosePhoto} style={{
              all: "unset", cursor: "pointer", padding: "9px 14px", borderRadius: 10,
              fontSize: 12.5, fontWeight: 600, color: "#FFF",
              background: "rgba(255,255,255,0.06)", border: "1px solid rgba(255,255,255,0.1)",
            }}>📷 Scegli foto</button>
            {(previewSrc) && (
              <button onClick={() => { setPendingPath(null); setClearPhoto(true); }} style={{
                all: "unset", cursor: "pointer", padding: "9px 14px", borderRadius: 10,
                fontSize: 12.5, fontWeight: 600, color: "rgba(237,233,255,0.7)",
                background: "transparent", border: "1px solid rgba(255,255,255,0.08)",
              }}>Rimuovi</button>
            )}
          </div>
        </div>

        <label style={{ display: "block", fontSize: 11, fontWeight: 700, letterSpacing: 1.2, color: "rgba(237,233,255,0.55)", textTransform: "uppercase", marginBottom: 8 }}>
          Nome
        </label>
        <input
          autoFocus
          value={name}
          onChange={e => setName(e.target.value)}
          placeholder="Es. Luca"
          style={{
            width: "100%", padding: "12px 16px", borderRadius: 12, boxSizing: "border-box",
            background: "rgba(255,255,255,0.05)", border: "1px solid rgba(255,255,255,0.1)",
            color: "#FFF", fontSize: 14, fontFamily: "var(--font-sans)", outline: "none",
          }}
        />

        {err && <div style={{ marginTop: 12, padding: "10px 12px", borderRadius: 10, background: "rgba(242,61,109,0.12)", color: "#F23D6D", fontSize: 12 }}>{err}</div>}

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 22 }}>
          <button onClick={onClose} style={{
            all: "unset", cursor: "pointer", padding: "10px 16px", borderRadius: 10,
            fontSize: 13, fontWeight: 600, background: "rgba(255,255,255,0.05)", color: "#EDE9FF",
            border: "1px solid rgba(255,255,255,0.08)",
          }}>Annulla</button>
          <button onClick={save} disabled={busy} style={{
            all: "unset", cursor: busy ? "default" : "pointer", padding: "10px 18px", borderRadius: 10,
            fontSize: 13, fontWeight: 600, background: CK_GRADIENT, color: "#FFF",
            opacity: busy ? 0.6 : 1,
          }}>{busy ? "Salvo…" : "Salva"}</button>
        </div>
      </div>
    </div>
  );
}

function PlayerCard({ player, stats, onEdit, onDelete, onView }) {
  const [hov, setHov] = useState(false);
  return (
    <div
      onMouseEnter={() => setHov(true)} onMouseLeave={() => setHov(false)}
      onClick={onView}
      style={{
        position: "relative", padding: 18, borderRadius: 18, cursor: "pointer",
        background: hov ? "rgba(255,255,255,0.05)" : "rgba(255,255,255,0.03)",
        border: "1px solid rgba(255,255,255,0.06)",
        transition: "all 160ms ease",
        display: "flex", flexDirection: "column", gap: 12,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
        <PhotoBubble player={player} size={64}/>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div style={{ fontFamily: "var(--font-display)", fontSize: 20, fontWeight: 700, color: "#FFF", letterSpacing: -0.4, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {player.name}
          </div>
          <div style={{ fontSize: 11, color: "rgba(237,233,255,0.45)", marginTop: 2 }}>
            {stats?.total_plays ?? 0} {stats?.total_plays === 1 ? "performance" : "performances"}
          </div>
        </div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 8 }}>
        <Stat label="Best" value={stats?.best_score ?? 0} accent="#FFD166"/>
        <Stat label="Media" value={stats ? Math.round(stats.avg_score) : 0} accent="#22D3A4"/>
        <Stat label="Brani" value={stats?.songs_played ?? 0} accent="#FF9070"/>
      </div>

      <div style={{ display: "flex", gap: 6, opacity: hov ? 1 : 0, transition: "opacity 140ms" }}>
        <button
          onClick={e => { e.stopPropagation(); onEdit(); }}
          style={{
            all: "unset", cursor: "pointer", flex: 1, textAlign: "center",
            padding: "8px 10px", borderRadius: 10, fontSize: 12, fontWeight: 600,
            background: "rgba(255,255,255,0.05)", color: "#EDE9FF",
            border: "1px solid rgba(255,255,255,0.08)",
          }}>Modifica</button>
        <button
          onClick={e => { e.stopPropagation(); onDelete(); }}
          style={{
            all: "unset", cursor: "pointer", flex: 1, textAlign: "center",
            padding: "8px 10px", borderRadius: 10, fontSize: 12, fontWeight: 600,
            background: "rgba(242,61,109,0.1)", color: "#F23D6D",
            border: "1px solid rgba(242,61,109,0.25)",
          }}>Elimina</button>
      </div>
    </div>
  );
}

function Stat({ label, value, accent }) {
  return (
    <div style={{
      padding: "8px 10px", borderRadius: 10, textAlign: "center",
      background: "rgba(255,255,255,0.03)", border: "1px solid rgba(255,255,255,0.05)",
    }}>
      <div style={{ fontFamily: "var(--font-display)", fontSize: 16, fontWeight: 700, color: accent, letterSpacing: -0.3 }}>
        {value}
      </div>
      <div style={{ fontSize: 9.5, fontWeight: 700, letterSpacing: 1, color: "rgba(237,233,255,0.45)", textTransform: "uppercase", marginTop: 2 }}>
        {label}
      </div>
    </div>
  );
}

function ProfileDetail({ player, onClose, onEdit }) {
  const [stats, setStats] = useState(null);
  const [history, setHistory] = useState([]);

  useEffect(() => {
    if (!player) return;
    invoke("players_stats", { id: player.id }).then(setStats).catch(() => {});
    invoke("players_history", { id: player.id, limit: 50 }).then(h => setHistory(Array.isArray(h) ? h : [])).catch(() => {});
  }, [player]);

  const formatDate = ts => ts ? new Date(ts * 1000).toLocaleDateString("it-IT") : "";

  return (
    <div style={{
      position: "fixed", inset: 0, background: "rgba(0,0,0,0.78)", backdropFilter: "blur(10px)",
      display: "flex", alignItems: "center", justifyContent: "center", zIndex: 1000, padding: 24,
    }} onClick={onClose}>
      <div style={{
        background: "#0D0B18", borderRadius: 22, padding: 28, width: "min(640px, 100%)", maxHeight: "85vh",
        border: "1px solid rgba(255,255,255,0.08)",
        boxShadow: "0 20px 60px rgba(0,0,0,0.6)",
        display: "flex", flexDirection: "column", overflow: "hidden",
      }} onClick={e => e.stopPropagation()}>
        <div style={{ display: "flex", alignItems: "center", gap: 16, marginBottom: 20 }}>
          <PhotoBubble player={player} size={84}/>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 1.6, color: "#FF9070", textTransform: "uppercase" }}>Profilo</div>
            <h2 style={{ margin: "4px 0 0", fontFamily: "var(--font-display)", fontSize: 30, fontWeight: 700, color: "#FFF", letterSpacing: -1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {player.name}
            </h2>
          </div>
          <button onClick={onEdit} style={{
            all: "unset", cursor: "pointer", padding: "9px 14px", borderRadius: 10,
            fontSize: 12.5, fontWeight: 600, background: "rgba(255,255,255,0.05)", color: "#EDE9FF",
            border: "1px solid rgba(255,255,255,0.08)",
          }}>Modifica</button>
        </div>

        <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 8, marginBottom: 18 }}>
          <Stat label="Plays" value={stats?.total_plays ?? 0} accent="#EDE9FF"/>
          <Stat label="Best" value={stats?.best_score ?? 0} accent="#FFD166"/>
          <Stat label="Media" value={stats ? Math.round(stats.avg_score) : 0} accent="#22D3A4"/>
          <Stat label="Brani" value={stats?.songs_played ?? 0} accent="#FF9070"/>
        </div>

        <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 1.4, color: "rgba(237,233,255,0.5)", textTransform: "uppercase", marginBottom: 8 }}>
          Cronologia
        </div>
        <div style={{ flex: 1, overflowY: "auto", borderRadius: 14, border: "1px solid rgba(255,255,255,0.05)" }}>
          {history.length === 0 ? (
            <div style={{ padding: 24, textAlign: "center", color: "rgba(237,233,255,0.4)", fontSize: 13 }}>
              Nessuna performance ancora.
            </div>
          ) : history.map((e, i) => (
            <div key={e.id || i} style={{
              display: "grid", gridTemplateColumns: "1fr auto auto", gap: 12, alignItems: "center",
              padding: "10px 14px", borderBottom: "1px solid rgba(255,255,255,0.04)", fontSize: 13,
            }}>
              <span style={{ color: "#FFF", fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{e.song_title}</span>
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 11.5, color: "rgba(237,233,255,0.5)" }}>{formatDate(e.created_at)}</span>
              <span style={{ fontFamily: "var(--font-display)", fontSize: 16, fontWeight: 700, color: "#FFF" }}>{e.score}</span>
            </div>
          ))}
        </div>

        <div style={{ display: "flex", justifyContent: "flex-end", marginTop: 18 }}>
          <button onClick={onClose} style={{
            all: "unset", cursor: "pointer", padding: "10px 18px", borderRadius: 10,
            fontSize: 13, fontWeight: 600, background: CK_GRADIENT, color: "#FFF",
          }}>Chiudi</button>
        </div>
      </div>
    </div>
  );
}

export default function Players() {
  const [players, setPlayers] = useState([]);
  const [statsById, setStatsById] = useState({});
  const [editing, setEditing] = useState(null); // null = closed; "new" or player object
  const [viewing, setViewing] = useState(null);

  const refresh = useCallback(async () => {
    try {
      const list = await invoke("players_list");
      const arr = Array.isArray(list) ? list : [];
      setPlayers(arr);
      const entries = await Promise.all(
        arr.map(async p => {
          try { return [p.id, await invoke("players_stats", { id: p.id })]; }
          catch { return [p.id, null]; }
        })
      );
      setStatsById(Object.fromEntries(entries));
    } catch (e) { console.error("players_list", e); }
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  const handleSaved = async () => {
    setEditing(null);
    await refresh();
  };

  const handleDelete = async (player) => {
    if (!confirm(`Eliminare il profilo di ${player.name}? I punteggi resteranno in classifica come anonimi.`)) return;
    try {
      await invoke("players_delete", { id: player.id });
      await refresh();
    } catch (e) { console.error(e); alert("Errore: " + e); }
  };

  return (
    <div style={{ padding: "28px 36px 36px", overflowY: "auto", height: "100%", boxSizing: "border-box" }}>
      <div style={{ display: "flex", alignItems: "flex-end", gap: 16, marginBottom: 24 }}>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 2, color: "#FF9070", textTransform: "uppercase", marginBottom: 8 }}>Profili</div>
          <h1 style={{ margin: 0, fontFamily: "var(--font-display)", fontWeight: 700, fontSize: 36, letterSpacing: -1.2, lineHeight: 1, color: "#FFF" }}>
            Giocatori
          </h1>
          <div style={{ marginTop: 8, fontSize: 13.5, color: "rgba(237,233,255,0.55)", fontWeight: 500 }}>
            Crea profili con foto per tenere traccia dei record. Pronti per i futuri duetti.
          </div>
        </div>
        <button onClick={() => setEditing("new")} style={{
          all: "unset", cursor: "pointer", padding: "12px 22px", borderRadius: 12,
          fontSize: 13, fontWeight: 700, background: CK_GRADIENT, color: "#FFF",
          boxShadow: "0 8px 22px rgba(242,61,109,0.4)",
        }}>+ Nuovo profilo</button>
      </div>

      {players.length === 0 ? (
        <div style={{
          padding: 60, borderRadius: 22, textAlign: "center",
          background: "rgba(255,255,255,0.03)", border: "1px dashed rgba(255,255,255,0.08)",
          color: "rgba(237,233,255,0.5)",
        }}>
          <div style={{ fontSize: 56, marginBottom: 14 }}>👥</div>
          <div style={{ fontFamily: "var(--font-display)", fontSize: 20, color: "#FFF", fontWeight: 700, marginBottom: 6 }}>
            Nessun profilo ancora
          </div>
          <div style={{ fontSize: 13, marginBottom: 20 }}>
            Crea il primo profilo per registrare i tuoi punteggi.
          </div>
          <button onClick={() => setEditing("new")} style={{
            all: "unset", cursor: "pointer", padding: "11px 22px", borderRadius: 12,
            fontSize: 13, fontWeight: 700, background: CK_GRADIENT, color: "#FFF",
            boxShadow: "0 8px 22px rgba(242,61,109,0.4)",
          }}>+ Crea profilo</button>
        </div>
      ) : (
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(260px, 1fr))", gap: 14 }}>
          {players.map(p => (
            <PlayerCard
              key={p.id}
              player={p}
              stats={statsById[p.id]}
              onEdit={() => setEditing(p)}
              onDelete={() => handleDelete(p)}
              onView={() => setViewing(p)}
            />
          ))}
        </div>
      )}

      {editing && (
        <EditorModal
          player={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={handleSaved}
        />
      )}
      {viewing && !editing && (
        <ProfileDetail
          player={viewing}
          onClose={() => setViewing(null)}
          onEdit={() => { setEditing(viewing); }}
        />
      )}
    </div>
  );
}
