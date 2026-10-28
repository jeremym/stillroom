import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { WatchPanel } from "../components/WatchPanel";
import { Glyph, Logo, Zone, applyLook, loadLook, type GlyphName, type Look } from "../kit";
import { PullModel } from "../components/PullModel";
import type { ModelsResponse, PermissionEntry, RecipesResponse, RuntimeStatus, Settings, WatchView } from "../types";

/* The Settings window (⌘,): what the app looks like, what it thinks with, where things live.
   Every change is saved as it is made and every window hears about it. */

type Section = "look" | "models" | "folders" | "watches" | "permissions" | "privacy" | "about";
const SECTIONS: { id: Section; label: string; glyph: GlyphName }[] = [
  { id: "look", label: "Look", glyph: "image" },
  { id: "models", label: "Models", glyph: "models" },
  { id: "folders", label: "Folders", glyph: "folder" },
  { id: "watches", label: "Watches", glyph: "watch" },
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
  { id: "embedding", label: "Embedding", what: "turns text into vectors for search" },
];

function lookOf(s: Settings): Look {
  const base = loadLook();
  return { theme: s.look?.theme ?? base.theme, size: s.look?.size ?? base.size, scanlines: s.look?.scanlines ?? base.scanlines };
}

export default function SettingsApp() {
  const [section, setSection] = useState<Section>("look");
  const [settings, setSettings] = useState<Settings>({});
  const [models, setModels] = useState<ModelsResponse | null>(null);
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [recipes, setRecipes] = useState<RecipesResponse | null>(null);
  const [watches, setWatches] = useState<WatchView[]>([]);
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
  const [keyDraft, setKeyDraft] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);
  const refreshKey = useCallback(() => { api.hasSecret("ollama.apiKey").then(setKeySet).catch(() => setKeySet(null)); }, []);

  const look = useMemo(() => lookOf(settings), [settings]);
  useEffect(() => applyLook(look), [look]);

  const refreshModels = useCallback(() => {
    setChecking(true);
    api.listModels().then(setModels).catch((e) => setError(String(e))).finally(() => setChecking(false));
  }, []);
  const refreshWatches = useCallback(() => { api.listWatches().then(setWatches).catch(() => {}); }, []);

  useEffect(() => {
    api.getSettings().then((s) => { setSettings(s); const u = s.ollama?.baseUrl ?? "http://localhost:11434"; setUrl(u); urlRef.current = u; }).catch((e) => setError(String(e)));
    api.runtimeStatus().then(setStatus).catch(() => {});
    api.listRecipes().then(setRecipes).catch(() => {});
    refreshModels();
    refreshWatches();
    refreshKey();
    const a = listen<Settings>("settings:changed", (ev) => setSettings(ev.payload));
    const c = listen("watch:changed", () => refreshWatches());
    return () => { a.then((f) => f()); c.then((f) => f()); };
  }, [refreshModels, refreshWatches, refreshKey]);

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

  const installed = models?.models ?? [];
  const withCap = (cap: string) => installed.filter((m) => m.capabilities.includes(cap as never));
  const pins = settings.pins ?? {};
  const pinnedRecipes = Object.entries(pins).filter(([, slots]) => Object.keys(slots ?? {}).length > 0);
  const provider = models?.providers[0];

  return (
    <div className={`settings-shell ${collapsed ? "collapsed" : ""}`}>
      <nav className="settings-nav" aria-label="Settings sections">
        <div className="settings-head"><Logo size={26} word={false} />{!collapsed && <h1>Settings</h1>}</div>
        {SECTIONS.map((s) => (
          <button key={s.id} aria-current={section === s.id} title={s.label} aria-label={s.label} onClick={() => setSection(s.id)}><Glyph name={s.glyph} />{!collapsed && <span>{s.label}</span>}</button>
        ))}
        <button className="nav-toggle" onClick={() => setCollapsed((c) => !c)} title={collapsed ? "Show section names" : "Icons only"} aria-label={collapsed ? "Show section names" : "Icons only"} aria-pressed={collapsed}>
          <Glyph name={collapsed ? "expand" : "collapse"} />{!collapsed && <span className="muted">Icons only</span>}
        </button>
      </nav>
      <div>
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
                <table>
                  <thead><tr><th>Model</th><th>Size</th><th>Context</th><th>Can</th></tr></thead>
                  <tbody>
                    {installed.map((m) => (
                      <tr key={m.id}>
                        <td className="mono">{m.name}</td>
                        <td>{m.parameterSize ?? ""}</td>
                        <td>{m.contextWindow ? `${Math.round(m.contextWindow / 1024)}k` : ""}</td>
                        <td><span className="caps">{m.capabilities.map((c) => <span key={c} className="tag">{c.replace(/_/g, " ")}</span>)}</span></td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              ) : provider?.available ? <p className="muted">Ollama is running but has no models. Pull one below.</p> : null}
            </Zone>

            <Zone title="Pull a model" sub="download through Ollama">
              <PullModel enabled={Boolean(provider?.available)} onDone={refreshModels} />
              <p className="muted">A small model (4 to 8 billion parameters) answers in seconds on most Macs; a large one is slower and sharper. Recipes pick by capability, so any model with the right abilities works.</p>
            </Zone>

            <Zone title="Defaults" sub="what a slot gets when it has no pin">
              <p className="muted">Order, decided before a run: your pin on the slot › your default for the slot's capability › the recipe's recommendation › the best installed model by this preference.</p>
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
                <span className="label" title="num_ctx when a recipe does not set one">Context length</span>
                <span className="row" style={{ margin: 0 }}>
                  <select value={settings.models?.contextLength ?? ""} style={{ width: 200 }} onChange={(e) => saveModels({ contextLength: e.target.value ? Number(e.target.value) : undefined })} aria-label="Default context length">
                    <option value="">Model's own default</option>
                    {[4096, 8192, 16384, 32768, 65536, 131072].map((n) => <option key={n} value={n}>{n >= 1024 ? `${n / 1024}k` : n} tokens</option>)}
                  </select>
                  <span className="muted">how much text a step can hold at once, when the recipe does not say · bigger is slower and uses more memory</span>
                </span>
                <span className="label" title="think when a recipe does not set it">Thinking</span>
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
                        {withCap(c.id).map((m) => <option key={m.id} value={m.id}>{m.name}{m.parameterSize ? ` · ${m.parameterSize}` : ""}</option>)}
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

            <Zone title="Cloud providers" sub="off">
              <p className="muted">Everything runs on this Mac and nothing leaves it. Cloud providers are not set up. When they arrive they will be opt-in here, keyed from the keychain, and every step that uses one will be marked amber on the map.</p>
            </Zone>
          </>
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
          </>
        )}

        {section === "permissions" && (
          <>
            <Zone title="What macOS may ask for" sub="and where each one stands" aside={<button className="quiet" onClick={() => loadPerms(true)}>Check the folders now</button>}>
              <p className="muted">Stillroom asks for very little. Folders are granted by macOS the first time a recipe or a watch touches them, with a system prompt. Nothing here is required to start.</p>
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
              <p className="muted">Models run in Ollama on this Mac, so there is no microphone, camera, screen, or accessibility access to grant. Files are read where you point, written to the recipe's own results, and the keychain holds only the optional access key for a remote Ollama.</p>
            </Zone>
          </>
        )}

        {section === "privacy" && (
          <>
            <Zone title="What leaves this Mac" sub="nothing, unless a recipe says so">
              <p>Models run in Ollama on this Mac. Files are read only where a recipe's permissions allow, and written only to its own results and the folders you chose.</p>
              <p>A recipe reaches the network only if it declares <span className="mono">permissions.network</span>. The usual grant is <span className="mono">input</span>: exactly the hosts in the links you typed, checked on every redirect.</p>
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
          <Zone title="About" sub="Stillroom 0.1, pre-alpha">
            <p>Describe the goal. Get the result. Open the door.</p>
            <p className="muted">{status ? `${status.schemaTitle} v0 · ${status.runCount} runs · tools: ${status.tools.join(", ")}` : "…"}</p>
            <p className="muted">Keyboard: ⌘0 Library · ⌘1 Recipe · ⌘2 Set up · ⌘3 Running · ⌘4 Results · ⌘5 Recipe card · ⌘N New recipe · ⌘R Run · ⌘, Settings · ⌘/ Help</p>
            <button onClick={() => save({ firstRun: true })}>{settings.firstRun ? "The welcome will show on next launch" : "Show the welcome on next launch"}</button>
          </Zone>
        )}

        {error && <pre className="error">{error}</pre>}
      </div>
    </div>
  );
}
