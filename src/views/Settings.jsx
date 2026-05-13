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

const inputStyle = {
  background: "rgba(255,255,255,0.04)",
  border: "1px solid rgba(255,255,255,0.08)",
  borderRadius: 8,
  padding: "6px 12px",
  fontSize: 12.5,
  color: "#FFF",
  outline: "none",
  fontFamily: "var(--font-sans)",
  width: 210,
};

function ToggleSwitch({ checked, onChange }) {
  return (
    <div onClick={onChange} style={{
      width: 40, height: 22, borderRadius: 11, cursor: "pointer", flexShrink: 0,
      background: checked ? CK_GRADIENT : "rgba(255,255,255,0.12)",
      border: "1px solid rgba(255,255,255,0.1)",
      position: "relative", transition: "background 200ms",
    }}>
      <div style={{
        position: "absolute", top: 2, left: checked ? 18 : 2,
        width: 16, height: 16, borderRadius: "50%",
        background: "#FFF", boxShadow: "0 2px 6px rgba(0,0,0,0.3)",
        transition: "left 200ms",
      }}/>
    </div>
  );
}

export default function Settings() {
  const [cookieBrowser, setCookieBrowser] = useState("safari");
  const [cookiesFile, setCookiesFile] = useState(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState(null);
  const [micDevices, setMicDevices] = useState([]);
  const [micDevice, setMicDevice] = useState(null); // null = system default

  // MEGA state
  const [megaEmail, setMegaEmail] = useState("");
  const [megaPassword, setMegaPassword] = useState("");
  const [megaMfa, setMegaMfa] = useState("");
  const [megaAutoSync, setMegaAutoSync] = useState(false);
  const [megaStatus, setMegaStatus] = useState(null); // { available, logged_in, account }
  const [megaMsg, setMegaMsg] = useState({ text: "", type: "idle" }); // type: idle|ok|error|loading
  const [megaSaving, setMegaSaving] = useState(false);

  useEffect(() => {
    invoke("get_cookie_browser").then(setCookieBrowser).catch(console.error);
    invoke("get_cookies_file").then(setCookiesFile).catch(console.error);
    invoke("list_mic_devices").then(setMicDevices).catch(console.error);
    invoke("get_mic_device").then(setMicDevice).catch(console.error);
    invoke("cloud_get_settings").then((s) => {
      setMegaEmail(s.email || "");
      setMegaPassword(s.password || "");
      setMegaMfa(s.mfa || "");
      setMegaAutoSync(s.auto_sync || false);
    }).catch(console.error);
    invoke("cloud_check_status").then(setMegaStatus).catch(console.error);
  }, []);

  async function selectMicDevice(name) {
    setSaving(true); setError(null);
    try {
      await invoke("set_mic_device", { name });
      setMicDevice(name);
    } catch (e) { setError(String(e)); } finally { setSaving(false); }
  }

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

  async function saveMegaCredentials() {
    setMegaSaving(true); setMegaMsg({ text: "Salvando...", type: "loading" });
    try {
      await invoke("cloud_save_credentials", { email: megaEmail, password: megaPassword, mfa: megaMfa, autoSync: megaAutoSync });
      setMegaMsg({ text: "Salvato", type: "ok" });
    } catch (e) {
      setMegaMsg({ text: String(e), type: "error" });
    } finally { setMegaSaving(false); }
  }

  async function megaLogin() {
    setMegaMsg({ text: "Login...", type: "loading" });
    try {
      const account = await invoke("cloud_login");
      setMegaStatus((prev) => ({ ...prev, logged_in: true, account }));
      setMegaMsg({ text: `Connesso come ${account}`, type: "ok" });
    } catch (e) { setMegaMsg({ text: String(e), type: "error" }); }
  }

  async function megaLogout() {
    try {
      await invoke("cloud_logout");
      setMegaStatus((prev) => ({ ...prev, logged_in: false, account: null }));
      setMegaMsg({ text: "Disconnesso", type: "ok" });
    } catch (e) { setMegaMsg({ text: String(e), type: "error" }); }
  }

  async function megaSyncAll() {
    setMegaMsg({ text: "Sync in corso...", type: "loading" });
    try {
      await invoke("cloud_sync_all");
      setMegaMsg({ text: "Sync completato", type: "ok" });
    } catch (e) { setMegaMsg({ text: String(e), type: "error" }); }
  }

  async function megaResyncAll() {
    setMegaMsg({ text: "Re-sync in corso (sostituzione file)...", type: "loading" });
    try {
      await invoke("cloud_resync_all");
      setMegaMsg({ text: "Re-sync completato", type: "ok" });
    } catch (e) { setMegaMsg({ text: String(e), type: "error" }); }
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

      {/* Mic selection */}
      <SettingGroup title="Microfono">
        {micDevices.length === 0 ? (
          <SettingRow label="Nessun microfono rilevato" hint="Controlla i permessi microfono nelle Impostazioni di sistema." last control={null} />
        ) : (
          <>
            <RadioCard
              label="Default di sistema"
              desc="Usa il microfono predefinito di macOS."
              checked={micDevice === null}
              disabled={saving}
              onChange={() => selectMicDevice(null)}
            />
            {micDevices.map((name, i) => (
              <RadioCard
                key={name}
                label={name}
                checked={micDevice === name}
                disabled={saving}
                onChange={() => selectMicDevice(name)}
                last={i === micDevices.length - 1}
              />
            ))}
          </>
        )}
      </SettingGroup>

      {/* MEGA cloud sync */}
      <SettingGroup title="MEGA.nz Cloud Sync">
        {/* MEGAcmd status indicator */}
        <SettingRow
          label="MEGAcmd"
          hint={
            megaStatus === null
              ? "Verifica in corso..."
              : megaStatus.available
              ? megaStatus.logged_in
                ? `Connesso come ${megaStatus.account}`
                : `Installato (${megaStatus.path}) — non autenticato`
              : "Non trovato. Installa con: brew install --cask megacmd"
          }
          control={
            <div style={{
              width: 10, height: 10, borderRadius: "50%", flexShrink: 0,
              background: megaStatus === null
                ? "rgba(255,255,255,0.3)"
                : megaStatus.available
                ? megaStatus.logged_in ? "#22D3A4" : "#FFB370"
                : "#F23D6D",
              boxShadow: megaStatus?.logged_in ? "0 0 8px #22D3A4" : "none",
            }}/>
          }
        />
        <SettingRow
          label="Email"
          hint="Account MEGA.nz"
          control={
            <input
              type="email"
              value={megaEmail}
              onChange={(e) => setMegaEmail(e.target.value)}
              placeholder="you@example.com"
              style={inputStyle}
            />
          }
        />
        <SettingRow
          label="Password"
          hint="Salvata in app_settings.json"
          control={
            <input
              type="password"
              value={megaPassword}
              onChange={(e) => setMegaPassword(e.target.value)}
              placeholder="••••••••"
              style={inputStyle}
            />
          }
        />
        <SettingRow
          label="Codice 2FA"
          hint="Solo se 2FA abilitato sull'account (TOTP a 6 cifre). Lascia vuoto se non usi 2FA."
          control={
            <input
              type="text"
              value={megaMfa}
              onChange={(e) => setMegaMfa(e.target.value)}
              placeholder="123456"
              maxLength={6}
              style={{ ...inputStyle, width: 90, fontFamily: "var(--font-mono)", letterSpacing: 3 }}
            />
          }
        />
        <SettingRow
          label="Auto-sync dopo processing"
          hint="Carica su MEGA automaticamente quando la pipeline finisce (incluso re-processing)"
          last
          control={
            <ToggleSwitch
              checked={megaAutoSync}
              onChange={() => setMegaAutoSync((v) => !v)}
            />
          }
        />
        <div style={{
          padding: "12px 18px",
          display: "flex", alignItems: "center", flexWrap: "wrap", gap: 8,
          borderTop: "1px solid rgba(255,255,255,0.04)",
        }}>
          <GhostBtn onClick={saveMegaCredentials} disabled={megaSaving}>
            {megaSaving ? "Salvando…" : "Salva"}
          </GhostBtn>
          {megaStatus?.available && !megaStatus.logged_in && (
            <GhostBtn onClick={megaLogin}>Login</GhostBtn>
          )}
          {megaStatus?.logged_in && (
            <>
              <GhostBtn onClick={megaLogout} danger>Logout</GhostBtn>
              <GhostBtn onClick={megaSyncAll}>Sync non sincronizzati</GhostBtn>
              <GhostBtn onClick={megaResyncAll}>Re-sync tutto</GhostBtn>
            </>
          )}
          {megaMsg.text && (
            <span style={{
              fontSize: 12,
              color: megaMsg.type === "ok" ? "#22D3A4"
                : megaMsg.type === "error" ? "#F23D6D"
                : "rgba(237,233,255,0.5)",
              flex: 1,
            }}>
              {megaMsg.text}
            </span>
          )}
        </div>
      </SettingGroup>

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
