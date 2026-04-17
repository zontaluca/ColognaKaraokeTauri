import logo from "../assets/logo.png";

const ITEMS = [
  { id: "library", label: "Library", icon: "🎵" },
  { id: "download", label: "Download", icon: "⬇️" },
  { id: "player", label: "Player", icon: "🎤" },
  { id: "leaderboard", label: "Classifica", icon: "🏆" },
];

export default function Sidebar({ view, onView }) {
  return (
    <aside className="sidebar">
      <div className="logo-box">
        <img src={logo} alt="Cologna Karaoke" />
        <div>
          <h1>Cologna Karaoke</h1>
        </div>
      </div>

      <nav className="nav">
        {ITEMS.map((it) => (
          <button
            key={it.id}
            className={`nav-btn ${view === it.id ? "active" : ""}`}
            onClick={() => onView(it.id)}
          >
            <span className="icon">{it.icon}</span>
            <span>{it.label}</span>
          </button>
        ))}
      </nav>

      <div className="sidebar-footer">v0.1 — local & offline</div>
    </aside>
  );
}
