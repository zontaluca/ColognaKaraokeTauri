import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

const CK_GRADIENT = "linear-gradient(135deg, #FFB370 0%, #FF6B5A 40%, #F23D6D 100%)";

function SettingGroup({ title, children }) {
  return (
    <div style={{ marginBottom: 28 }}>
      <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 1.4, color: "#FF9070", textTransform: "uppercase", marginBottom: 10 }}>
        {title}
      </div>
      <div style={{ borderRadius: 16, overflow: "hidden", background: "rgba(255,255,255,0.03)", border: "1px solid rgba(255,255,255,0.06)" }}>
        {children}
      </div>
    </div>
  );
}

function SettingRow({ label, hint, control, last }) {
  return (
    <div style={{
      display: "flex", alignItems: "center", gap: 16,
      padding: "14px 18px",
      borderBottom: last ? "none" : "1px solid rgba(255,255,255,0.04)",
    }}>
      <div style={{ flex: 1 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: "#FFF" }}>{label}</div>
        {hint && <div style={{ fontSize: 11.5, color: "rgba(237,233,255,0.5)", marginTop: 2 }}>{hint}</div>}
      </div>
      {control}
    </div>
  );
}

function RadioCard({ label, desc, checked, disabled, onChange }) {
  return (
    <label style={{
      display: "flex", alignItems: "flex-start", gap: 12,
      padding: "12px 18px", cursor: disabled ? "default" : "pointer",
      borderBottom: "1px solid rgba(255,255,255,0.04)",
      background: checked ? "rgba(255,107,90,0.06)" : "transparent",
      transition: "background 140ms",
    }}>
      <div style={{
        width: 16, height: 16, borderRadius: "50%", flexShrink: 0, marginTop: 2,
        border: `2px solid ${checked ? "#FF6B5A" : "rgba(255,255,255,0.2)"}`,
        background: checked ? CK_GRADIENT : "transparent",
        display: "flex", alignItems: "center", justifyContent: "center",
        transition: "all 140ms",
      }}>
        {checked && <div style={{ width: 5, height: 5, borderRadius: "50%", background: "#FFF" }}/>}
      </div>
      <input type="radio" checked={checked} disabled={disabled} onChange={onChange} style={{ display: "none" }}/>
      <div>
        <div style={{ fontSize: 13, fontWeight: 600, color: "#FFF" }}>{label}</div>
        {desc && <div style={{ fontSize: 11.5, color: "rgba(237,233,255,0.5)", marginTop: 3, lineHeight: 1.5 }} dangerouslySetInnerHTML={{ __html: desc }}/>}
      </div>
    </label>
  );
}

function GhostBtn({ children, onClick, danger }) {
  return (
    <button onClick={onClick} style={{
      all: "unset", cursor: "pointer",
      padding: "7px 14px", borderRadius: 9, fontSize: 12.5, fontWeight: 600,
      background: danger ? "rgba(242,61,109,0.1)" : "rgba(255,255,255,0.05)",
      color: danger ? "#F23D6D" : "#EDE9FF",
      border: `1px solid ${danger ? "rgba(242,61,109,0.25)" : "rgba(255,255,255,0.08)"}`,
      transition: "all 140ms",
    }}>{children}</button>
  );
}

export default function Settings() {
  const [cookieBrowser, setCookieBrowser] = useState("safari");
  const [cookiesFile, setCookiesFile] = useState(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState(null);

  useEffect(() => {
    invoke("get_cookie_browser").then(setCookieBrowser).catch(console.error);
    invoke("get_cookies_file").then(setCookiesFile).catch(console.error);
  }, []);

  async function selectCookieBrowser(val) {
    setSaving(true); setError(null);
    try { await invoke("set_cookie_browser", { browser: val }); setCookieBrowser(val); }
    catch (e) { setError(String(e)); } finally { setSaving(false); }
  }

  async function pickCookiesFile() {
    try {
      const path = await open({ filters: [{ name: "Cookies", extensions: ["txt"] }] });
      if (path) { await invoke("set_cookies_file", { path }); setCookiesFile(path); }
    } catch (e) { setError(String(e)); }
  }

  async function clearCookiesFile() {
    try { await invoke("set_cookies_file", { path: null }); setCookiesFile(null); }
    catch (e) { setError(String(e)); }
  }

  return (
    <div style={{ padding: "28px 36px 36px", maxWidth: 760, overflowY: "auto", height: "100%", boxSizing: "border-box" }}>
      {/* Header */}
      <div style={{ marginBottom: 32 }}>
        <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: 2, color: "#FF9070", textTransform: "uppercase", marginBottom: 8 }}>Preferences</div>
        <h1 style={{ margin: 0, fontFamily: "var(--font-display)", fontWeight: 700, fontSize: 36, letterSpacing: -1.2, lineHeight: 1, color: "#FFF" }}>
          Settings
        </h1>
        <div style={{ marginTop: 8, fontSize: 13.5, color: "rgba(237,233,255,0.55)", fontWeight: 500 }}>
          Tune the app to your room and your voice.
        </div>
      </div>

      {/* YouTube cookies section */}
      <SettingGroup title="YouTube cookies">
        <SettingRow
          label="Cookies file"
          hint="Export cookies.txt via the «Get cookies.txt LOCALLY» Chrome extension, then select it here."
          control={
            <div style={{ display: "flex", alignItems: "center", gap: 8, flexShrink: 0 }}>
              {cookiesFile && (
                <>
                  <span style={{ fontSize: 11, color: "rgba(237,233,255,0.5)", fontFamily: "var(--font-mono)", background: "rgba(255,255,255,0.04)", padding: "4px 8px", borderRadius: 6, maxWidth: 180, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {cookiesFile.split("/").pop()}
                  </span>
                  <GhostBtn onClick={clearCookiesFile} danger>✕</GhostBtn>
                </>
              )}
              <GhostBtn onClick={pickCookiesFile}>
                {cookiesFile ? "Change…" : "Select cookies.txt"}
              </GhostBtn>
            </div>
          }
        />
        {!cookiesFile && (
          <div>
            <div style={{ padding: "10px 18px 4px", fontSize: 11, fontWeight: 700, letterSpacing: 1.2, color: "rgba(237,233,255,0.4)", textTransform: "uppercase" }}>
              Or extract from browser (may fail due to macOS sandbox)
            </div>
            {[
              { val: "safari",   label: "Safari" },
              { val: "chrome",   label: "Chrome" },
              { val: "firefox",  label: "Firefox" },
              { val: "chromium", label: "Chromium" },
              { val: "none",     label: "None — no cookies" },
            ].map(({ val, label }, i, arr) => (
              <RadioCard
                key={val} label={label} checked={cookieBrowser === val}
                disabled={saving} onChange={() => selectCookieBrowser(val)}
                last={i === arr.length - 1}
              />
            ))}
          </div>
        )}
      </SettingGroup>

    </div>
  );
}
