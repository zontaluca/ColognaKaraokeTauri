const CK_GRADIENT = "linear-gradient(135deg, #FFB370 0%, #FF6B5A 40%, #F23D6D 100%)";

function BrandMark() {
  return (
    <div style={{
      width: 38, height: 38, borderRadius: 11,
      background: CK_GRADIENT, flexShrink: 0,
      display: "flex", alignItems: "center", justifyContent: "center",
      boxShadow: "0 6px 18px rgba(242,61,109,0.45), inset 0 1px 0 rgba(255,255,255,0.3)",
    }}>
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none">
        <rect x="9" y="3" width="6" height="11" rx="3" fill="#FFF"/>
        <path d="M6 11a6 6 0 0012 0" stroke="#FFF" strokeWidth="1.8" strokeLinecap="round"/>
        <path d="M12 17v3" stroke="#FFF" strokeWidth="1.8" strokeLinecap="round"/>
        <circle cx="12" cy="8" r="1" fill="#F23D6D"/>
      </svg>
    </div>
  );
}

function WaveBar({ delay }) {
  return (
    <div style={{
      width: 3, background: "#FFF", borderRadius: 2,
      animation: `wavePulse 900ms ease-in-out ${delay}ms infinite alternate`,
      height: "40%",
    }}/>
  );
}

function NavIcon({ id, active }) {
  const c = active ? "#FFF" : "rgba(237,233,255,0.65)";
  const w = 18, h = 18;
  if (id === "library") return (
    <svg width={w} height={h} viewBox="0 0 24 24" fill="none">
      <path d="M9 18V5l10-2v13" stroke={c} strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"/>
      <circle cx="6" cy="18" r="3" stroke={c} strokeWidth="1.8"/>
      <circle cx="16" cy="16" r="3" stroke={c} strokeWidth="1.8"/>
    </svg>
  );
  if (id === "download") return (
    <svg width={w} height={h} viewBox="0 0 24 24" fill="none">
      <path d="M12 4v11m0 0l-4-4m4 4l4-4" stroke={c} strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"/>
      <path d="M5 19h14" stroke={c} strokeWidth="1.8" strokeLinecap="round"/>
    </svg>
  );
  if (id === "player") return (
    <svg width={w} height={h} viewBox="0 0 24 24" fill="none">
      <rect x="9" y="3" width="6" height="11" rx="3" stroke={c} strokeWidth="1.8"/>
      <path d="M6 11a6 6 0 0012 0M12 17v3M9 21h6" stroke={c} strokeWidth="1.8" strokeLinecap="round"/>
    </svg>
  );
  if (id === "leaderboard") return (
    <svg width={w} height={h} viewBox="0 0 24 24" fill="none">
      <path d="M7 4h10v5a5 5 0 01-10 0V4z" stroke={c} strokeWidth="1.8" strokeLinejoin="round"/>
      <path d="M17 6h3v2a3 3 0 01-3 3M7 6H4v2a3 3 0 003 3M10 18h4v3h-4zM8 21h8" stroke={c} strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"/>
    </svg>
  );
  if (id === "settings") return (
    <svg width={w} height={h} viewBox="0 0 24 24" fill="none">
      <circle cx="12" cy="12" r="3" stroke={c} strokeWidth="1.8"/>
      <path d="M19.4 15a1.6 1.6 0 00.3 1.8l.1.1a2 2 0 11-2.8 2.8l-.1-.1a1.6 1.6 0 00-1.8-.3 1.6 1.6 0 00-1 1.5V21a2 2 0 11-4 0v-.1a1.6 1.6 0 00-1-1.5 1.6 1.6 0 00-1.8.3l-.1.1a2 2 0 11-2.8-2.8l.1-.1a1.6 1.6 0 00.3-1.8 1.6 1.6 0 00-1.5-1H3a2 2 0 110-4h.1a1.6 1.6 0 001.5-1 1.6 1.6 0 00-.3-1.8l-.1-.1a2 2 0 112.8-2.8l.1.1a1.6 1.6 0 001.8.3H9a1.6 1.6 0 001-1.5V3a2 2 0 114 0v.1a1.6 1.6 0 001 1.5 1.6 1.6 0 001.8-.3l.1-.1a2 2 0 112.8 2.8l-.1.1a1.6 1.6 0 00-.3 1.8V9a1.6 1.6 0 001.5 1H21a2 2 0 110 4h-.1a1.6 1.6 0 00-1.5 1z" stroke={c} strokeWidth="1.5"/>
    </svg>
  );
  return null;
}

const ITEMS = [
  { id: "library",     label: "Library"    },
  { id: "download",    label: "Download"   },
  { id: "player",      label: "Player"     },
  { id: "leaderboard", label: "Classifica" },
  { id: "settings",    label: "Settings"   },
];

export default function Sidebar({ view, onView }) {
  return (
    <aside style={{
      width: 240, height: "100vh", flexShrink: 0,
      padding: 14, display: "flex", flexDirection: "column", gap: 10,
      borderRight: "1px solid rgba(255,255,255,0.04)",
      background: "linear-gradient(180deg, rgba(255,107,90,0.04) 0%, rgba(7,6,12,0) 60%)",
      boxSizing: "border-box",
    }}>
      {/* Brand */}
      <div style={{
        display: "flex", alignItems: "center", gap: 12,
        padding: "14px 12px", borderRadius: 16,
        background: "linear-gradient(135deg, rgba(255,107,90,0.12), rgba(242,61,109,0.06))",
        border: "1px solid rgba(255,107,90,0.18)",
        flexShrink: 0,
      }}>
        <BrandMark />
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          <div style={{ fontFamily: "var(--font-display)", fontWeight: 700, fontSize: 14, letterSpacing: -0.2, color: "#FFF" }}>
            Cologna
          </div>
          <div style={{ fontSize: 10.5, fontWeight: 600, letterSpacing: 1.4, color: "#FF9070", textTransform: "uppercase" }}>
            Karaoke
          </div>
        </div>
      </div>

      {/* Nav */}
      <nav style={{ display: "flex", flexDirection: "column", gap: 4, marginTop: 6 }}>
        {ITEMS.map(({ id, label }) => {
          const active = view === id;
          return (
            <button key={id}
              onClick={() => onView(id)}
              style={{
                all: "unset", cursor: "pointer",
                display: "flex", alignItems: "center", gap: 12,
                padding: "11px 14px", borderRadius: 12,
                fontSize: 13.5, fontWeight: 600, letterSpacing: -0.1,
                color: active ? "#FFF" : "rgba(237,233,255,0.72)",
                background: active ? CK_GRADIENT : "transparent",
                boxShadow: active ? "0 8px 24px rgba(242,61,109,0.35), inset 0 1px 0 rgba(255,255,255,0.25)" : "none",
                transition: "all 160ms ease",
                position: "relative",
              }}
              onMouseEnter={e => { if (!active) e.currentTarget.style.background = "rgba(255,255,255,0.04)"; }}
              onMouseLeave={e => { if (!active) e.currentTarget.style.background = "transparent"; }}
            >
              <NavIcon id={id} active={active}/>
              {label}
              {active && (
                <div style={{ marginLeft: "auto", width: 6, height: 6, borderRadius: "50%", background: "#FFF", boxShadow: "0 0 8px rgba(255,255,255,0.7)" }}/>
              )}
            </button>
          );
        })}
      </nav>

      <div style={{ flex: 1 }}/>

      {/* Mini now-playing */}
      <button onClick={() => onView("player")} style={{
        all: "unset", cursor: "pointer",
        padding: 10, borderRadius: 14,
        background: "rgba(255,255,255,0.03)",
        border: "1px solid rgba(255,255,255,0.06)",
        display: "flex", alignItems: "center", gap: 10,
      }}>
        <div style={{
          width: 40, height: 40, borderRadius: 10, flexShrink: 0,
          background: "linear-gradient(135deg, #FF9A76 0%, #FF4F76 60%, #A240FF 100%)",
          display: "flex", alignItems: "center", justifyContent: "center",
          boxShadow: "0 4px 12px rgba(242,61,109,0.4)",
        }}>
          <div style={{ display: "flex", alignItems: "center", gap: 2, height: 18 }}>
            {[0, 110, 220, 330].map((delay, i) => <WaveBar key={i} delay={delay}/>)}
          </div>
        </div>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div style={{ fontSize: 12, fontWeight: 600, color: "#FFF", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            Now Playing
          </div>
          <div style={{ fontSize: 10.5, color: "rgba(237,233,255,0.5)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            Tap to open player
          </div>
        </div>
      </button>

      {/* Footer */}
      <div style={{
        padding: "10px 12px", borderRadius: 10,
        fontSize: 10, fontWeight: 600, letterSpacing: 0.5,
        color: "rgba(237,233,255,0.35)",
        display: "flex", alignItems: "center", justifyContent: "space-between",
      }}>
        <span>v0.2 · local & offline</span>
        <span style={{ color: "#22D3A4", fontSize: 8 }}>●</span>
      </div>
    </aside>
  );
}
