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
  const [whisperModel, setWhisperModel] = useState(null);
  const [alignmentMode, setAlignmentMode] = useState("forced_per_phrase");
  const [micDevices, setMicDevices] = useState([]);
  const [micDevice, setMicDevice] = useState(null); // null = system default

  useEffect(() => {
    invoke("get_cookie_browser").then(setCookieBrowser).catch(console.error);
    invoke("get_cookies_file").then(setCookiesFile).catch(console.error);
    invoke("get_whisper_model").then(setWhisperModel).catch(console.error);
    invoke("get_alignment_mode").then(setAlignmentMode).catch(console.error);
    invoke("list_mic_devices").then(setMicDevices).catch(console.error);
    invoke("get_mic_device").then(setMicDevice).catch(console.error);
  }, []);

  async function selectMicDevice(name) {
    setSaving(true); setError(null);
    try {
      await invoke("set_mic_device", { name });
      setMicDevice(name);
    } catch (e) { setError(String(e)); } finally { setSaving(false); }
  }

  const gpuDisabled = whisperModel === "disabled";

  async function selectAlignmentMode(val) {
    setSaving(true); setError(null);
    try { await invoke("set_alignment_mode", { mode: val }); setAlignmentMode(val); }
    catch (e) { setError(String(e)); } finally { setSaving(false); }
  }

  async function selectWhisperModel(val) {
    setSaving(true); setError(null);
    try { await invoke("set_whisper_model", { model: val }); setWhisperModel(val); }
    catch (e) { setError(String(e)); } finally { setSaving(false); }
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

      {/* Word alignment */}
      <SettingGroup title="Word alignment">
        {gpuDisabled ? (
          <SettingRow
            label="Whisper model"
            hint="GPU non disponibile — allineamento word-level disattivato. Player usa timing per linea."
            last
            control={
              <span style={{
                fontSize: 12, fontFamily: "var(--font-mono)",
                padding: "4px 10px", borderRadius: 7,
                background: "rgba(255,255,255,0.06)",
                color: "rgba(237,233,255,0.75)",
                border: "1px solid rgba(255,255,255,0.08)",
                flexShrink: 0,
              }}>Disabled</span>
            }
          />
        ) : (
          <>
            <div style={{ padding: "10px 18px 4px", fontSize: 11, fontWeight: 700, letterSpacing: 1.2, color: "rgba(237,233,255,0.4)", textTransform: "uppercase" }}>
              Modello Whisper
            </div>
            {[
              {
                val: "large_v3_turbo",
                label: "Large V3 Turbo",
                desc: "Più accurato. Più lento sui brani lunghi. Default raccomandato.",
              },
              {
                val: "medium",
                label: "Medium",
                desc: "Più veloce. Accuratezza inferiore su brani con allucinazioni o lyrics non standard.",
              },
            ].map(({ val, label, desc }, i, arr) => (
              <RadioCard
                key={val} label={label} desc={desc}
                checked={whisperModel === val}
                disabled={saving}
                onChange={() => selectWhisperModel(val)}
                last={i === arr.length - 1}
              />
            ))}
            <div style={{ padding: "10px 18px 4px", fontSize: 11, fontWeight: 700, letterSpacing: 1.2, color: "rgba(237,233,255,0.4)", textTransform: "uppercase" }}>
              Algoritmo di allineamento
            </div>
            {[
              {
                val: "forced_per_phrase",
                label: "Forced alignment",
                desc: "Whisper allinea il testo LRC noto all'audio per frase. Veloce, richiede LRC corrispondente al cantato.",
              },
              {
                val: "free_transcribe_per_phrase",
                label: "Free transcribe + match",
                desc: "Whisper trascrive ogni frase liberamente, fuzzy-match con LRC. Più lento ma tollera mismatch e allucinazioni.",
              },
            ].map(({ val, label, desc }, i, arr) => (
              <RadioCard
                key={val} label={label} desc={desc}
                checked={alignmentMode === val}
                disabled={saving}
                onChange={() => selectAlignmentMode(val)}
                last={i === arr.length - 1}
              />
            ))}
          </>
        )}
      </SettingGroup>

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
