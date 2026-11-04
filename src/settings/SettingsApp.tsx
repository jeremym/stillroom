import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { WatchPanel } from "../components/WatchPanel";
import { Glyph, Logo, type GlyphName } from "../brand";
import { Tag, Toggle, Zone, applyLook, fmtElapsed, loadLook, useElapsed, type Look } from "@overtone-foundry/app-kit";
import { rememberProviders, whereWord } from "../models";
import { measuredTag } from "../planWith";
import { PullModel } from "../components/PullModel";
import { PromptEditor } from "../components/PromptEditor";
import { fitOf, fitWord, fmtSpeed, speedOf, thisMacLine } from "../hardware";
import type { CodexStatus, Hardware, InstalledModel, ModelsResponse, PermissionEntry, PlanMeasurement, RecipesResponse, RuntimeStatus, Schedule, Settings, TryPlanningResult, TryResult, UpdateCheck, WatchView } from "../types";
import { whenWord } from "../schedules";
import { updateWord } from "../updates";

/* The Settings window (⌘,): what the app looks like, what it thinks with, where things live.
   Every change is saved as it is made and every window hears about it. */

type Section = "look" | "models" | "prompts" | "folders" | "watches" | "permissions" | "privacy" | "about";
const SECTIONS: { id: Section; label: string; glyph: GlyphName }[] = [
  { id: "look", label: "Look", glyph: "image" },
  { id: "models", label: "Models", glyph: "models" },
  { id: "prompts", label: "Prompts", glyph: "chat" },
  { id: "folders", label: "Folders", glyph: "folder" },
  { id: "watches", label: "Watches and schedules", glyph: "watch" },
  { id: "permissions", label: "Permissions", glyph: "checklist" },
  { id: "privacy", label: "Privacy", glyph: "shield" },
  { id: "about", label: "About", glyph: "help" },
];
const PERM_WORD: Record<PermissionEntry["status"], string> = {
  granted: "granted", denied: "not allowed", unknown: "not asked yet", not_needed: "not needed", in_use: "in use", not_in_use: "not in use",
};
const CAPABILITIES: { id: string; label: string; what: string }[] = [
  { id: "text", label: "Text", what: "reads and writes prose" },
  { id: "structured_output", label: "Structured output", what: "answers in a fixed shape (JSON)" },
  { id: "tool_calling", label: "Tool calling", what: "can call the recipe's tools (agents)" },
  { id: "thinking", label: "Thinking", what: "reasons before answering" },
  { id: "vision", label: "Vision", what: "reads images" },
  { id: "embedding", label: "Embedding", what: "turns text into numbers for search (embeddings)" },
];

function lookOf(s: Settings): Look {
  const base = loadLook();
  return { theme: s.look?.theme ?? base.theme, size: s.look?.size ?? base.size, scanlines: s.look?.scanlines ?? base.scanlines };
}

export default function SettingsApp() {
  const [section, setSection] = useState<Section>("look");
  const [settings, setSettings] = useState<Settings>({});
  const [models, setModels] = useState<ModelsResponse | null>(null);
  // This Mac: read once from the engine the first time Models is opened; what could not be read is null.
  const [hardware, setHardware] = useState<Hardware | null>(null);
  useEffect(() => { if (section === "models" && !hardware) api.hardware().then(setHardware).catch(() => {}); }, [section, hardware]);
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [recipes, setRecipes] = useState<RecipesResponse | null>(null);
  const [watches, setWatches] = useState<WatchView[]>([]);
  const [schedules, setSchedules] = useState<Schedule[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [url, setUrl] = useState("");
  const [checking, setChecking] = useState(false);
  const urlRef = useRef("");
  const [collapsed, setCollapsed] = useState<boolean>(() => {
    try { const v = localStorage.getItem("p1.settingsNav"); if (v) return v === "collapsed"; } catch { /* per-viewer */ }
    return window.innerWidth < 820;
  });
  useEffect(() => { try { localStorage.setItem("p1.settingsNav", collapsed ? "collapsed" : "open"); } catch { /* per-viewer */ } }, [collapsed]);
  const [keySet, setKeySet] = useState<boolean | null>(null);
  const [perms, setPerms] = useState<PermissionEntry[] | null>(null);
  const loadPerms = useCallback((probe: boolean) => { api.permissionStatus(probe).then(setPerms).catch((e) => setError(String(e))); }, []);
  useEffect(() => { if (section === "permissions" && !perms) loadPerms(false); }, [section, perms, loadPerms]);
  const [codex, setCodex] = useState<CodexStatus | null>(null);
  const [codexDraft, setCodexDraft] = useState("");
  const [codexPath, setCodexPath] = useState("");
  const refreshCodex = useCallback(() => { api.codexStatus().then(setCodex).catch((e) => setError(String(e))); }, []);
  useEffect(() => { if (section === "models") refreshCodex(); }, [section, refreshCodex, settings.codex?.path]);
  const [oaUrl, setOaUrl] = useState("");
  const [oaDraft, setOaDraft] = useState("");
  const [oaKeyDraft, setOaKeyDraft] = useState("");
  const [oaKeySet, setOaKeySet] = useState<boolean | null>(null);
  const [oaKeyBusy, setOaKeyBusy] = useState(false);
  const refreshOaKey = useCallback(() => { api.hasSecret("openai.apiKey").then(setOaKeySet).catch(() => setOaKeySet(null)); }, []);
  useEffect(() => { if (section === "models") refreshOaKey(); }, [section, refreshOaKey]);
  useEffect(() => { setOaUrl(settings.openai?.baseUrl ?? ""); }, [settings.openai?.baseUrl]);
  const [tries, setTries] = useState<Record<string, { busy: boolean; result: TryResult | null }>>({});
  const tryModel = useCallback(async (id: string) => {
    setTries((t) => ({ ...t, [id]: { busy: true, result: null } }));
    try {
      const result = await api.tryModel(id);
      setTries((t) => ({ ...t, [id]: { busy: false, result } }));
      // The engine remembered the speed; the Speed column reads it from the models list.
      if (result.completionTokensPerSecond) api.listModels().then((m) => setModels((prev) => (prev ? { ...prev, speeds: m.speeds } : m))).catch(() => {});
    } catch (e) {
      setTries((t) => ({ ...t, [id]: { busy: false, result: { ok: false, durationMs: 0, answer: null, error: String(e), promptTokens: null, completionTokens: null } } }));
    }
  }, []);
  // Try planning: the planner's first call on a sample goal, timed and checked. The engine records the
  // result under planner.measured and says settings:changed, which is how the row's tag updates.
  const [planTries, setPlanTries] = useState<Record<string, PlanTry>>({});
  const tryPlanning = useCallback(async (id: string) => {
    setPlanTries((t) => ({ ...t, [id]: { since: new Date().toISOString() } }));
    try {
      const result = await api.tryPlanning(id);
      setPlanTries((t) => ({ ...t, [id]: { result } }));
    } catch (e) {
      setPlanTries((t) => ({ ...t, [id]: { error: String(e) } }));
    }
  }, []);
  const planCell = (m: InstalledModel) => <TryPlanningCell model={m} state={planTries[m.id]} measured={settings.planner?.measured?.[m.id]} onTry={tryPlanning} />;
  const [keyDraft, setKeyDraft] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);
  const refreshKey = useCallback(() => { api.hasSecret("ollama.apiKey").then(setKeySet).catch(() => setKeySet(null)); }, []);

  const look = useMemo(() => lookOf(settings), [settings]);
  useEffect(() => applyLook(look), [look]);

  const refreshModels = useCallback(() => {
    setChecking(true);
    api.listModels().then((m) => { rememberProviders(m.providers); setModels(m); }).catch((e) => setError(String(e))).finally(() => setChecking(false));
  }, []);
  const refreshWatches = useCallback(() => { api.listWatches().then(setWatches).catch(() => {}); }, []);
  // An engine without schedules refuses the call; then there are none to show.
  const refreshSchedules = useCallback(() => { api.listSchedules().then(setSchedules).catch(() => setSchedules([])); }, []);

  useEffect(() => {
    api.getSettings().then((s) => { setSettings(s); const u = s.ollama?.baseUrl ?? "http://localhost:11434"; setUrl(u); urlRef.current = u; }).catch((e) => setError(String(e)));
    api.runtimeStatus().then(setStatus).catch(() => {});
    api.listRecipes().then(setRecipes).catch(() => {});
    refreshModels();
    refreshWatches();
    refreshSchedules();
    refreshKey();
    const a = listen<Settings>("settings:changed", (ev) => setSettings(ev.payload));
    const c = listen("watch:changed", () => refreshWatches());
    const d = listen("schedule:changed", () => refreshSchedules());
    return () => { a.then((f) => f()); c.then((f) => f()); d.then((f) => f()); };
  }, [refreshModels, refreshWatches, refreshSchedules, refreshKey]);

  async function saveKey(value: string) {
    setKeyBusy(true);
    setError(null);
    try {
      await api.setSecret("ollama.apiKey", value);
      setKeyDraft("");
      refreshKey();
      refreshModels();
    } catch (e) {
      setError(String(e));
    } finally {
      setKeyBusy(false);
    }
  }

  async function save(patch: Record<string, unknown>) {
    setError(null);
    try {
      setSettings(await api.setSettings(patch));
    } catch (e) {
      setError(String(e));
    }
  }
  const saveLook = (next: Partial<Look>) => save({ look: { ...settings.look, ...next } });
  const saveModels = (next: Partial<NonNullable<Settings["models"]>>) => save({ models: { ...settings.models, ...next } });

  async function applyUrl() {
    let u = url.trim().replace(/\/+$/, "") || "http://localhost:11434";
    if (!/^https?:\/\//i.test(u)) u = `http://${u}`;
    // A password in the address would land in plain SQLite. Strip it and point at the key field.
    try {
      const parsed = new URL(u);
      if (parsed.username || parsed.password) {
        parsed.username = "";
        parsed.password = "";
        u = parsed.toString().replace(/\/+$/, "");
        setError("The address had a user name or password in it. That part was dropped: use the Access key field below, which keeps secrets in the keychain.");
      }
    } catch {
      setError(`That does not look like an address: ${u}`);
      return;
    }
    setUrl(u);
    await save({ ollama: { baseUrl: u } });
    refreshModels();
  }

  const allModels = models?.models ?? [];
  const installed = allModels.filter((m) => m.provider === "ollama");
  const codexModels = allModels.filter((m) => m.provider === "codex");
  const codexOn = Boolean(settings.codex?.enabled);
  const openaiModels = allModels.filter((m) => m.provider === "openai");
  const openaiStatus = models?.providers.find((p) => p.id === "openai");
  const openaiOn = Boolean(settings.openai?.enabled);
  const saveOpenai = (next: Partial<NonNullable<Settings["openai"]>>) => save({ openai: { ...settings.openai, ...next } }).then(refreshModels);
  async function saveOaKey(value: string) {
    setOaKeyBusy(true);
    try { await api.setSecret("openai.apiKey", value); setOaKeyDraft(""); refreshOaKey(); refreshModels(); } catch (e) { setError(String(e)); } finally { setOaKeyBusy(false); }
  }
  const saveCodex = (next: Partial<NonNullable<Settings["codex"]>>) => save({ codex: { ...settings.codex, ...next } }).then(refreshModels);
  const withCap = (cap: string) => installed.filter((m) => m.capabilities.includes(cap as never));
  const pins = settings.pins ?? {};
  const pinnedRecipes = Object.entries(pins).filter(([, slots]) => Object.keys(slots ?? {}).length > 0);
  const provider = models?.providers[0];

  return (
    <div className={`settings-shell ${collapsed ? "collapsed" : ""}`}>
      <nav className="settings-nav" aria-label="Settings sections">
        <div className="settings-head"><Logo size={26} word={false} />{!collapsed && <h1>Settings</h1>}
          <button className="nav-toggle" onClick={() => setCollapsed((c) => !c)} title={collapsed ? "Show section names" : "Icons only"} aria-label={collapsed ? "Show section names" : "Icons only"} aria-pressed={collapsed}>
            <Glyph name={collapsed ? "expand" : "collapse"} />
          </button>
        </div>
        {SECTIONS.map((s) => (
          <button key={s.id} aria-current={section === s.id} title={s.label} aria-label={s.label} onClick={() => setSection(s.id)}><Glyph name={s.glyph} />{!collapsed && <span>{s.label}</span>}</button>
        ))}
      </nav>
      <div className="settings-main">
        {section === "look" && (
          <Zone title="Look" sub="the same three themes everywhere">
            <div className="settings-row">
              <span className="label">Theme</span>
              <span className="segmented" role="group" aria-label="Theme">
                {(["green", "amber", "paper"] as const).map((t) => <button key={t} aria-pressed={look.theme === t} onClick={() => saveLook({ theme: t })}>{t}</button>)}
              </span>
              <span className="label">Text size</span>
              <span className="segmented" role="group" aria-label="Text size">
                {(["normal", "larger", "largest"] as const).map((s) => <button key={s} aria-pressed={look.size === s} onClick={() => saveLook({ size: s })}>{s}</button>)}
              </span>
              <span className="label">Scanlines</span>
              <span>
                <button className="toggle" aria-pressed={look.scanlines} onClick={() => saveLook({ scanlines: !look.scanlines })}>
                  <span className={look.scanlines ? "on" : ""}>on</span><span className={look.scanlines ? "" : "on"}>off</span>
                </button>
                <span className="muted"> · faint lines over the screen, off unless you like them</span>
              </span>
            </div>
            <p className="muted">Green is the machine, paper is what it made, amber means it needs you or something would leave this Mac. Paper is the high-contrast theme.</p>
          </Zone>
        )}

        {section === "models" && (
          <>
            <Zone title="This Mac" sub="what the models run on">
              {thisMacLine(hardware) ? (
                <>
                  <p className="this-mac"><strong>This Mac</strong> · {thisMacLine(hardware)}</p>
                  <p className="muted this-mac-note">A model larger than that still runs, but slowly: it is swapped through memory. Speed below is what each model managed last time on this Mac.</p>
                </>
              ) : (
                <p className="muted this-mac-note">{hardware ? "This Mac did not say what it is made of." : "Reading what this Mac is made of…"} Speed below is what each model managed last time on this Mac.</p>
              )}
            </Zone>

            <Zone title="Ollama" sub="the local model runner" aside={provider && <><span className={`dot ${provider.available ? "ok" : "bad"}`} />{provider.available ? `${installed.length} models` : "not running"}</>}>
              <div className="row field-row">
                <div className="field">
                  <label><strong>Address</strong> <span className="muted">· where Ollama answers; the default is this Mac</span></label>
                  <input type="text" className="mono" value={url} onChange={(e) => setUrl(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") applyUrl(); }} placeholder="http://localhost:11434" />
                </div>
                <button onClick={applyUrl} disabled={checking}>{checking ? "Checking…" : "Save and check"}</button>
              </div>
              {provider && !provider.available && <p className="muted">Nothing answered at <span className="mono">{provider.baseUrl}</span>. Start Ollama, or install it from ollama.com, then check again.</p>}
              <div className="row field-row">
                <div className="field">
                  <label><strong><Glyph name="key" /> Access key</strong> <span className="muted">· optional · for an Ollama behind a proxy that wants a bearer token · kept in the macOS keychain, never shown again</span></label>
                  <input type="password" className="mono" value={keyDraft} onChange={(e) => setKeyDraft(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && keyDraft.trim()) saveKey(keyDraft); }}
                    placeholder={keySet ? "a key is set · type a new one to replace it" : "no key · leave empty for a local Ollama"} autoComplete="off" disabled={keyBusy} />
                </div>
                <button onClick={() => saveKey(keyDraft)} disabled={keyBusy || !keyDraft.trim()}>Save key</button>
                {keySet && <button className="danger" onClick={() => saveKey("")} disabled={keyBusy}>Remove key</button>}
              </div>
              <p className="muted">Sent as <span className="mono">Authorization: Bearer …</span> on every call to the address above. A local Ollama needs none.</p>
              {installed.length > 0 ? (
                <>
                  <p className="muted planner-note">Not every model plans well; the planner (what drafts a recipe from a goal) uses the fastest clean one unless you pin another. <strong>Try planning</strong> measures a model on a sample goal.</p>
                  <table>
                    <thead><tr><th>Model</th><th>Size</th><th title="How the weights sit in this Mac's memory: room under 40 %, tight to 60 %, too large past that">Fit</th><th>Context</th><th title="Tokens per second the model managed last time on this Mac">Speed</th><th>Can</th><th></th><th></th></tr></thead>
                    <tbody>
                      {installed.map((m) => (
                        <tr key={m.id}>
                          <td className="mono">{m.name}{m.loaded ? <> <span className="tag live">in memory</span></> : null}</td>
                          <td>{[m.parameterSize, m.quantization].filter(Boolean).join(" · ")}</td>
                          <td className={fitOf(m.sizeBytes, hardware?.memoryBytes) === "too_large" ? "amber" : undefined}>{m.local ? fitWord(fitOf(m.sizeBytes, hardware?.memoryBytes)) : "cloud"}</td>
                          <td>{m.contextWindow ? `${Math.round(m.contextWindow / 1024)}k` : ""}</td>
                          <td className="speed">{speedOf(models?.speeds, m.id)}</td>
                          <td><span className="caps">{m.capabilities.map((c) => <span key={c} className="tag">{c.replace(/_/g, " ")}</span>)}</span></td>
                          <td><TryCell id={m.id} state={tries[m.id]} onTry={tryModel} /></td>
                          <td>{planCell(m)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </>
              ) : provider?.available ? <p className="muted">Ollama is running but has no models. Pull one below.</p> : null}
            </Zone>

            <Zone title="Pull a model" sub={installed.length ? `download another through Ollama · you have ${installed.length}` : "download through Ollama"}>
              <PullModel enabled={Boolean(provider?.available)} installed={installed.map((m) => m.name)} onDone={refreshModels} />
              <p className="muted">A small model (4 to 8 billion parameters) answers in seconds on most Macs; a large one is slower and sharper. Recipes ask for abilities (capabilities), not model names, so any model that has them works.</p>
            </Zone>

            <Zone title="Defaults" sub="what a slot gets when it has no pin">
              <p className="muted">Order, decided before a run: your pin on the slot › your default for that kind of step › the recipe's recommendation › the best installed model by this preference.</p>
              <div className="defaults">
                <span className="label">Prefer</span>
                <span className="segmented" role="group" aria-label="Preference">
                  {(["accuracy", "speed"] as const).map((p) => <button key={p} aria-pressed={(settings.models?.preference ?? "accuracy") === p} onClick={() => saveModels({ preference: p })}>{p}</button>)}
                </span>
                <span className="label" title="How long one model call may take">Time limit</span>
                <span className="row" style={{ margin: 0 }}>
                  <input type="number" min={0} step={30} style={{ width: 120 }} value={settings.models?.timeoutSecs ?? 600}
                    onChange={(e) => saveModels({ timeoutSecs: Math.max(0, Number(e.target.value) || 0) })} aria-label="Seconds per model call" />
                  <span className="muted">seconds per model call · 0 means wait forever · a stuck call fails the step instead of hanging the run</span>
                </span>
                <span className="label" title="when a recipe does not set one (num_ctx)">Context length</span>
                <span className="row" style={{ margin: 0 }}>
                  <select value={settings.models?.contextLength ?? ""} style={{ width: 200 }} onChange={(e) => saveModels({ contextLength: e.target.value ? Number(e.target.value) : undefined })} aria-label="Default context length">
                    <option value="">Model's own default</option>
                    {[4096, 8192, 16384, 32768, 65536, 131072].map((n) => <option key={n} value={n}>{n >= 1024 ? `${n / 1024}k` : n} tokens</option>)}
                  </select>
                  <span className="muted">how much text a step can hold at once, when the recipe does not say · bigger is slower and uses more memory</span>
                </span>
                <span className="label" title="when a recipe does not say (think)">Thinking</span>
                <span className="row" style={{ margin: 0 }}>
                  <span className="segmented" role="group" aria-label="Thinking">
                    {([["", "Model's default"], ["off", "Off"], ["on", "On"]] as const).map(([v, label]) => (
                      <button key={v} aria-pressed={(settings.models?.think == null ? "" : settings.models.think ? "on" : "off") === v}
                        onClick={() => saveModels({ think: v === "" ? null : v === "on" })}>{label}</button>
                    ))}
                  </span>
                  <span className="muted">reasoning before answering, for models that can · off is faster; writers rarely need it</span>
                </span>
                {CAPABILITIES.map((c) => (
                  <span key={c.id} style={{ display: "contents" }}>
                    <span className="label" title={c.what}>{c.label}</span>
                    <span>
                      <select value={settings.models?.byCapability?.[c.id] ?? ""} onChange={(e) => {
                        const next = { ...(settings.models?.byCapability ?? {}) };
                        if (e.target.value) next[c.id] = e.target.value; else delete next[c.id];
                        saveModels({ byCapability: next });
                      }}>
                        <option value="">Auto · {c.what}</option>
                        {withCap(c.id).map((m) => <option key={m.id} value={m.id}>{m.name}{m.parameterSize ? ` · ${m.parameterSize}` : ""}{m.local ? "" : ` · ${whereWord(m.id)}`}</option>)}
                      </select>
                    </span>
                  </span>
                ))}
              </div>
            </Zone>

            <Zone title="Pins" sub="models you chose for a recipe's slots" aside={<>{pinnedRecipes.length} recipe{pinnedRecipes.length === 1 ? "" : "s"}</>}>
              {pinnedRecipes.length === 0 ? <p className="muted">No pins. Pick a model on a slot in Set up or on the map and it shows up here.</p> : (
                <ul className="rows">
                  {pinnedRecipes.map(([rid, slots]) => (
                    <li key={rid}>
                      <strong>{recipes?.recipes.find((r) => r.id === rid)?.name ?? rid}</strong>
                      <div className="row">
                        {Object.entries(slots).map(([slot, model]) => (
                          <span key={slot} className="fact"><span className="muted">{slot}:</span> <span className="mono">{model}</span>
                            <button className="quiet" onClick={() => { const next = { ...slots }; delete next[slot]; save({ pins: { ...pins, [rid]: next } }); }}>clear</button></span>
                        ))}
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </Zone>

            <Zone title="Codex" sub="cloud models through your ChatGPT plan" className={codexOn ? "accent" : undefined}
              aside={codex && <><span className={`dot ${codex.installed && codex.signedIn ? "ok" : "bad"}`} />{!codex.installed ? "not installed" : !codex.signedIn ? "not signed in" : codexOn ? "on" : "off"}</>}>
              <p className="muted">Stillroom can hand an AI step to the <strong>Codex app</strong> (the command <span className="mono">codex exec</span>, run headless in a read-only sandbox). It is billed to the ChatGPT plan Codex is signed in with, never per token. <span className="amber">The prompt and the step's inputs leave this Mac</span>; every step on a Codex model is marked amber. Auto never picks one: you choose it as a pin or a default.</p>
              {codex && (
                <ul className="rows">
                  <li><strong>Program</strong> <span className="muted">{codex.installed ? <>{codex.version ?? "found"} · <span className="mono">{codex.path}</span></> : "not found. Install the Codex app or CLI, then come back."}</span></li>
                  <li><strong>Signed in</strong> <span className="muted">{!codex.installed ? "—" : codex.signedIn ? "yes" : <>no. In Terminal, run <span className="mono">codex login</span>, then check again.</>}</span></li>
                  <li><strong>Default model</strong> <span className="muted mono">{codex.defaultModel ?? "none set in ~/.codex/config.toml"}</span></li>
                </ul>
              )}
              <div className="row field-row">
                <div className="field">
                  <label><strong>Use Codex models</strong> <span className="muted">· off keeps Codex out of every list</span></label>
                  <Toggle on={codexOn} onChange={(on) => saveCodex({ enabled: on })} label="Use Codex models" disabled={!codex?.installed || !codex?.signedIn} />
                </div>
                <button onClick={() => { refreshCodex(); refreshModels(); }}>Check again</button>
              </div>
              {!codex?.installed && (
                <div className="row field-row">
                  <div className="field">
                    <label><strong>Where the program is</strong> <span className="muted">· only if Stillroom cannot find it on its own</span></label>
                    <input type="text" className="mono" value={codexPath} onChange={(e) => setCodexPath(e.target.value)} placeholder="/path/to/codex" />
                  </div>
                  <button onClick={() => saveCodex({ path: codexPath.trim() || undefined })}>Save</button>
                </div>
              )}
              {codexOn && (
                <>
                  <table>
                    <thead><tr><th>Model</th><th>Can</th><th></th><th></th><th></th></tr></thead>
                    <tbody>
                      {codexModels.map((m) => (
                        <tr key={m.id}>
                          <td className="mono">{m.name}{m.name === codex?.defaultModel ? <span className="muted"> · Codex's default</span> : ""}</td>
                          <td><span className="caps">{m.capabilities.map((c) => <span key={c} className="tag">{c.replace(/_/g, " ")}</span>)}</span></td>
                          <td><TryCell id={m.id} state={tries[m.id]} onTry={tryModel} /></td>
                          <td>{planCell(m)}</td>
                          <td>{m.name !== codex?.defaultModel && <button className="link" onClick={() => saveCodex({ models: (settings.codex?.models ?? []).filter((n) => n !== m.name) })}>remove</button>}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                  <div className="row field-row">
                    <div className="field">
                      <label><strong>Add a model</strong> <span className="muted">· a name Codex accepts; Try tells you whether it answers</span></label>
                      <input type="text" className="mono" value={codexDraft} onChange={(e) => setCodexDraft(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && codexDraft.trim()) { saveCodex({ models: [...(settings.codex?.models ?? []), codexDraft.trim()] }); setCodexDraft(""); } }} placeholder="gpt-5-codex" />
                    </div>
                    <button onClick={() => { if (codexDraft.trim()) { saveCodex({ models: [...(settings.codex?.models ?? []), codexDraft.trim()] }); setCodexDraft(""); } }} disabled={!codexDraft.trim()}>Add</button>
                  </div>
                  <p className="muted">Each call carries Codex's own instructions, about 13k tokens, before your prompt. Good for recipe steps, wasteful for chatter. Codex steps cannot use recipe tools, so agent steps stay local.</p>
                </>
              )}
            </Zone>

            <Zone title="OpenAI by API key" sub="or any service that speaks the same language" className={openaiOn && openaiStatus && !openaiStatus.local ? "accent" : undefined}
              aside={<><span className={`dot ${openaiOn && openaiStatus?.available ? "ok" : "bad"}`} />{!openaiOn ? "off" : openaiStatus?.available ? `${openaiModels.length} models` : "not answering"}</>}>
              <p className="muted">Pay-per-token access with a key: OpenAI itself, or anything with the same API, such as LM Studio or llama-server on this Mac. <span className="amber">A cloud address means prompts and inputs leave this Mac</span> and every such step is marked amber; an address on this Mac counts as local. Auto never picks a cloud model.</p>
              <p className="muted">Checked by the author only against a local OpenAI-compatible server. If OpenAI's own service misbehaves, say so.</p>
              <div className="row field-row">
                <div className="field">
                  <label><strong>Use these models</strong> <span className="muted">· off keeps them out of every list</span></label>
                  <Toggle on={openaiOn} onChange={(on) => saveOpenai({ enabled: on })} label="Use OpenAI-compatible models" />
                </div>
                <button onClick={refreshModels}>Check again</button>
              </div>
              {openaiOn && (
                <>
                  <div className="row field-row">
                    <div className="field">
                      <label><strong>Address</strong> <span className="muted">· the API base, ending in /v1</span></label>
                      <input type="text" className="mono" value={oaUrl} onChange={(e) => setOaUrl(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") saveOpenai({ baseUrl: oaUrl.trim() || undefined }); }} placeholder="https://api.openai.com/v1" />
                    </div>
                    <button onClick={() => saveOpenai({ baseUrl: oaUrl.trim() || undefined })}>Save and check</button>
                  </div>
                  {openaiStatus && !openaiStatus.available && <p className="muted">Nothing answered at <span className="mono">{openaiStatus.baseUrl}</span>. Check the address and the key.</p>}
                  <div className="row field-row">
                    <div className="field">
                      <label><strong><Glyph name="key" /> API key</strong> <span className="muted">· kept in the macOS keychain, never shown again · a local server may need none</span></label>
                      <input type="password" className="mono" value={oaKeyDraft} onChange={(e) => setOaKeyDraft(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && oaKeyDraft.trim()) saveOaKey(oaKeyDraft); }}
                        placeholder={oaKeySet ? "a key is set · type a new one to replace it" : "no key"} autoComplete="off" disabled={oaKeyBusy} />
                    </div>
                    <button onClick={() => saveOaKey(oaKeyDraft)} disabled={oaKeyBusy || !oaKeyDraft.trim()}>Save key</button>
                    {oaKeySet && <button className="danger" onClick={() => saveOaKey("")} disabled={oaKeyBusy}>Remove key</button>}
                  </div>
                  {openaiModels.length > 0 && (
                    <table>
                      <thead><tr><th>Model</th><th>Can</th><th></th><th></th><th></th></tr></thead>
                      <tbody>
                        {openaiModels.map((m) => (
                          <tr key={m.id}>
                            <td className="mono">{m.name}</td>
                            <td><span className="caps">{m.capabilities.map((c) => <span key={c} className="tag">{c.replace(/_/g, " ")}</span>)}</span></td>
                            <td><TryCell id={m.id} state={tries[m.id]} onTry={tryModel} /></td>
                            <td>{planCell(m)}</td>
                            <td>{(settings.openai?.models ?? []).includes(m.name) && <button className="link" onClick={() => saveOpenai({ models: (settings.openai?.models ?? []).filter((n) => n !== m.name) })}>remove</button>}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  )}
                  <div className="row field-row">
                    <div className="field">
                      <label><strong>Models</strong> <span className="muted">· {(settings.openai?.models ?? []).length ? "the names you listed; Try tells you whether each answers" : "none listed, so every model the service reports is offered · list names to keep it short"}</span></label>
                      <input type="text" className="mono" value={oaDraft} onChange={(e) => setOaDraft(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && oaDraft.trim()) { saveOpenai({ models: [...(settings.openai?.models ?? []), oaDraft.trim()] }); setOaDraft(""); } }} placeholder="gpt-5" />
                    </div>
                    <button onClick={() => { if (oaDraft.trim()) { saveOpenai({ models: [...(settings.openai?.models ?? []), oaDraft.trim()] }); setOaDraft(""); } }} disabled={!oaDraft.trim()}>Add</button>
                  </div>
                  <p className="muted">The service does not say what each model can do, so every listed model is offered for text, fixed-shape answers, tools, and thinking. Pick accordingly.</p>
                </>
              )}
            </Zone>
          </>
        )}

        {section === "prompts" && (
          <Zone title="Prompts" sub="the words the app itself runs on">
            <p>These are the instructions (system prompts) Stillroom gives a model for its own jobs. A recipe's prompts live in the recipe; these sit underneath. Read them, change them, keep every revision, go back to the original whenever you like.</p>
            <h3>Strip <span className="muted">(the first planning call: which stages)</span></h3>
            <PromptEditor name="strip" onError={(m) => setError(m)} />
            <h3>Planner <span className="muted">(the second call: the steps of custom stages)</span></h3>
            <PromptEditor name="planner" onError={(m) => setError(m)} />
            <p className="quiet-line plans-journal">Every plan is written down as it happens: the goal, the model, which prompt revision was in use, what the checker found on each attempt, and everything the model wrote.
              <button className="link" onClick={() => api.revealPlansFolder().catch((e) => setError(String(e)))}>Open the planner's journal</button></p>
          </Zone>
        )}

        {section === "folders" && (
          <Zone title="Folders" sub="where recipes and results live">
            {status ? (
              <>
                {status.recipeRoots.map((r, i) => (
                  <p key={r}><span className="status-word muted">{i === 0 ? "Built-in recipes" : "Your recipes"}</span><br /><span className="mono">{r}</span></p>
                ))}
                <p className="muted">Results and the run history sit beside your recipes in <span className="mono">stillroom.db</span> and an <span className="mono">artifacts</span> folder.</p>
                <button onClick={() => api.revealRecipesFolder().catch((e) => setError(String(e)))}>Show my recipes folder</button>
              </>
            ) : <p className="muted">…</p>}
          </Zone>
        )}

        {section === "watches" && (
          <>
            {watches.length === 0 ? (
              <Zone title="Folder watches" sub="recipes that run on their own while Stillroom is open">
                <p className="muted">No watches. In a recipe's Set up, choose a folder input and press Watch this folder.</p>
              </Zone>
            ) : (
              <>
                <div className="row" style={{ marginTop: 0 }}>
                  <button onClick={() => Promise.all(watches.map((w) => api.setWatchEnabled(w.id, false))).then(refreshWatches).catch((e) => setError(String(e)))}>Pause all</button>
                  <button onClick={() => Promise.all(watches.map((w) => api.setWatchEnabled(w.id, true))).then(refreshWatches).catch((e) => setError(String(e)))}>Resume all</button>
                </div>
                <WatchPanel watches={watches} onChanged={refreshWatches} onOpenRun={() => {}} onError={(m) => setError(m)} />
              </>
            )}
            <Zone title="Schedules" sub="recipes that run at a time of day, while Stillroom is open">
              {schedules.length === 0 ? (
                <p className="muted">No schedules. In a recipe's Set up, press Run every day at… to run it at the same time each day. Turn one on or off, run it now, or delete it from the Library.</p>
              ) : (
                <>
                  <ul className="watches schedules">
                    {schedules.map((s) => (
                      <li key={s.id}>
                        <strong>{s.recipeName}</strong>
                        <span className="muted"> {whenWord(s.time, s.days)}</span>
                        <Tag tone={s.enabled ? "live" : undefined}>{s.enabled ? "on" : "off"}</Tag>
                      </li>
                    ))}
                  </ul>
                  <p className="muted">Turn a schedule on or off, run it now, or delete it from the Library.</p>
                </>
              )}
            </Zone>
          </>
        )}

        {section === "permissions" && (
          <>
            <Zone title="What macOS may ask for" sub="and where each one stands" aside={<button className="quiet" onClick={() => loadPerms(true)}>Check the folders now</button>}>
              <p className="muted">Stillroom asks for very little. Folders are granted by macOS the first time a recipe or a watch touches them, with a system prompt.</p>
              <p className="notice" role="note"><Glyph name="yesno" /> Nothing here is required to start. You can close this and run a recipe right now.</p>
              {!perms ? <p className="muted">…</p> : (
                <ul className="rows perms">
                  {perms.map((p) => (
                    <li key={p.id} className={p.need === "not_needed" ? "dim" : ""}>
                      <div className="perm-row">
                        <span className="perm-main">
                          <strong>{p.label}</strong>
                          <span className={`tag ${p.status === "granted" || p.status === "in_use" ? "live" : p.status === "denied" ? "bad" : ""}`}>{PERM_WORD[p.status]}</span>
                          {p.need === "optional" && <span className="tag">optional</span>}
                          <br /><span className="muted">{p.why}</span>
                        </span>
                        <span className="perm-act">
                          {p.need === "not_needed"
                            ? <button disabled title="Stillroom does not use this, so there is nothing to grant">Not needed</button>
                            : p.status === "granted" || p.status === "in_use"
                              ? <button disabled title="Already granted">Granted</button>
                              : p.pane
                                ? <button onClick={() => api.openPrivacyPane(p.pane!).catch((e) => setError(String(e)))} title="Opens System Settings at the right pane">Open System Settings…</button>
                                : <button disabled title="macOS asks when it is first used">Asked when used</button>}
                        </span>
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </Zone>
            <Zone title="Why so few" sub="local-first, by design">
              <p className="muted">Models run in Ollama on this Mac{codexOn ? ", or through the Codex app when you choose a Codex model," : ""} so there is no microphone, camera, screen, or accessibility access to grant. Files are read where you point, written to the recipe's own results, and the keychain holds only the optional access key for a remote Ollama.</p>
            </Zone>
          </>
        )}

        {section === "privacy" && (
          <>
            <Zone title="What leaves this Mac" sub="nothing, unless a recipe says so">
              <p>Models run in Ollama on this Mac. Files are read only where a recipe's permissions allow, and written only to its own results and the folders you chose.</p>
              {codexOn && <p className="amber"><Glyph name="warning" /> Codex is on: a step you put on a Codex model sends its prompt and inputs to OpenAI through your ChatGPT plan. Such steps are marked amber on the map and in Set up.</p>}
              {openaiOn && openaiStatus && !openaiStatus.local && <p className="amber"><Glyph name="warning" /> An API-key service at <span className="mono">{openaiStatus.baseUrl}</span> is on: a step you put on one of its models sends its prompt and inputs there. Such steps are marked amber.</p>}
              {!codexOn && !(openaiOn && openaiStatus && !openaiStatus.local) && <p className="muted">No cloud provider is on, so no model step leaves this Mac.</p>}
              <p>A recipe reaches the network only if it declares <span className="mono">permissions.network</span>. The usual grant is <span className="mono">input</span>: exactly the hosts in the links you typed, checked on every redirect.</p>
            </Zone>
            <Zone title="Remembered answers" sub="what Reuse unchanged steps draws on">
              <p className="muted">Every AI step's answer is kept on this Mac with the prompt, model, settings, and inputs that made it, so a later run with exactly the same can reuse it instead of asking the model again (selective rerun). Nothing here leaves the Mac. Forgetting them makes every step of every recipe run fresh next time.</p>
              <ForgetAnswers />
            </Zone>
            <Zone title="Recipes that may use the network">
              {recipes ? (() => {
                const net = recipes.recipes.filter((r) => r.permissions?.network);
                return net.length === 0 ? <p className="muted">None.</p> : (
                  <ul className="rows">{net.map((r) => <li key={r.id}><strong>{r.name}</strong> <span className="muted mono">· {JSON.stringify(r.permissions?.network)}</span></li>)}</ul>
                );
              })() : <p className="muted">…</p>}
            </Zone>
          </>
        )}

        {section === "about" && (
          <>
            <Zone title={status ? `Stillroom ${status.version}` : "Stillroom"} sub={status && status.version.startsWith("0.") ? "an early release: the format can still change" : undefined}>
              <p>Describe the goal. Get the result. See how it was made.</p>
              <p className="muted">{status ? `${status.schemaTitle} v0 · ${status.runCount} runs · tools: ${status.tools.join(", ")}` : "…"}</p>
              <p className="muted">Keyboard: ⌘0 Library · ⌘1 Recipe · ⌘2 Set up · ⌘3 Running · ⌘4 Results · ⌘5 Recipe card · ⌘N New recipe · ⌘R Run · ⌘, Settings · ⌘/ Help</p>
              <button onClick={() => save({ firstRun: true })}>{settings.firstRun ? "The first-run screen will show on next launch" : "Show the first-run screen on next launch"}</button>
            </Zone>
            <Zone title="Updates" sub="asks GitHub for the newest release number; nothing is downloaded or installed">
              <UpdatesPanel last={settings.updates?.last} daily={settings.updates?.daily === true}
                onDaily={(on) => save({ updates: { ...settings.updates, daily: on } })} onError={setError} />
            </Zone>
          </>
        )}

        {error && <pre className="error">{error}</pre>}
      </div>
    </div>
  );
}

/* One tiny call to a model from the table: the answer time, or what went wrong. */
function TryCell({ id, state, onTry }: { id: string; state?: { busy: boolean; result: TryResult | null }; onTry: (id: string) => void }) {
  const r = state?.result;
  return (
    <span className="try-cell">
      <button className="link" onClick={() => onTry(id)} disabled={state?.busy}>{state?.busy ? "trying…" : r ? "try again" : "Try"}</button>
      {r && (r.ok
        ? <span className="muted"> · answered in {(r.durationMs / 1000).toFixed(1)} s{r.completionTokensPerSecond ? <> · {fmtSpeed(r.completionTokensPerSecond)}</> : ""}{r.answer ? <> · “{r.answer}”</> : ""}</span>
        : <span className="red"> · {r.error}</span>)}
    </span>
  );
}

/** A trial plan from the table: running since, or what it came back with. */
type PlanTry = { since: string } | { result: TryPlanningResult } | { error: string };

/* Try planning, for a model that answers in a fixed shape (JSON): the planner's first call on a sample goal,
   timed and checked; up to five minutes, Stop cancels. The engine remembers the result, and the row wears it
   as a tag ("plans in 48 s" / "plan had problems") until the next trial. */
function TryPlanningCell({ model, state, measured, onTry }: { model: InstalledModel; state?: PlanTry; measured?: PlanMeasurement; onTry: (id: string) => void }) {
  const since = state && "since" in state ? state.since : null;
  const elapsed = useElapsed(since, Boolean(since));
  if (!model.capabilities.includes("structured_output")) return null;
  const tag = measuredTag(measured);
  return (
    <span className="try-cell try-planning">
      {since ? (
        <>
          <span className="muted">Trying… {fmtElapsed(elapsed ?? 0)}</span>
          <button className="link" onClick={() => api.stopRun("plan").catch(() => {})}><Glyph name="stop" /> Stop</button>
        </>
      ) : (
        <button className="link" onClick={() => onTry(model.id)} title="Run the planner's first call on a sample goal with this model, and time it">{measured || state ? "Try planning again" : "Try planning"}</button>
      )}
      {tag && !since && <Tag tone={tag.amber ? "amber" : undefined}>{tag.text}</Tag>}
      {state && "error" in state && <span className="red"> · {/stopped|cancel/i.test(state.error) ? "stopped" : state.error}</span>}
      {state && "result" in state && !state.result.clean && state.result.problems[0] && <span className="muted"> · {state.result.problems[0]}</span>}
    </span>
  );
}

/* Settings › About › Updates: Check now, what the last check found, and the daily check. The check is
   the one call Stillroom makes beyond this Mac on its own, so it is off until turned on here. */
export function UpdatesPanel({ last, daily, onDaily, onError }: { last?: UpdateCheck; daily: boolean; onDaily: (on: boolean) => void; onError: (e: string | null) => void }) {
  const [busy, setBusy] = useState(false);
  const [showNotes, setShowNotes] = useState(false);
  const word = updateWord(last);
  async function checkNow() {
    setBusy(true);
    onError(null);
    try {
      await api.checkForUpdates();
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <p className={word.tone} role="status" data-testid="update-word">
        {word.text}
        {last?.newer && last.url && <> <button className="link" onClick={() => api.openLink(last.url!).catch((e) => onError(String(e)))}>download from GitHub</button></>}
        {last?.newer && last.notes && <> · <button className="link" onClick={() => setShowNotes((v) => !v)}>{showNotes ? "hide what changed" : "what changed"}</button></>}
      </p>
      {showNotes && last?.notes && <pre className="muted update-notes">{last.notes}</pre>}
      <div className="row">
        <button onClick={checkNow} disabled={busy}>{busy ? "Checking…" : "Check now"}</button>
        <Toggle on={daily} onChange={onDaily} label="Check once a day when Stillroom opens" />
      </div>
      <p className="muted">Either way this is one request to GitHub's release feed for this repository, with no account and nothing about you or your recipes in it. Installed through Homebrew? <span className="mono">brew upgrade stillroom</span> does the same from the Terminal.</p>
    </>
  );
}

/** Settings › Privacy: forget every remembered step answer, and say how many went. */
function ForgetAnswers() {
  const [busy, setBusy] = useState(false);
  const [word, setWord] = useState<string | null>(null);
  return (
    <div className="row">
      <button disabled={busy} onClick={() => {
        setBusy(true);
        setWord(null);
        api.clearStepCache().then((n) => setWord(n === 0 ? "Nothing was remembered." : `Forgot ${n} answer${n === 1 ? "" : "s"}; the next run of every recipe asks its models again.`)).catch((e) => setWord(String(e))).finally(() => setBusy(false));
      }}>Forget remembered answers</button>
      {word && <span className="muted">{word}</span>}
    </div>
  );
}
