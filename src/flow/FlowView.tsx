import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Background, BaseEdge, Handle, Position, ReactFlow, type Connection, type Edge, type EdgeProps, type EdgeTypes, type NodeProps, type NodeTypes } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { api } from "../api";
import { buildGraph, type FlowNode } from "./graph";
import { StepDetails } from "./StepDetails";
import { addStep, connect, disconnect, docToInfo, mutate, removeStep, stepPathOf } from "./draft";
import { Glyph, fmtElapsed, inputGlyph, outputGlyph, stepGlyph, useElapsed } from "../kit";
import {
  AddStepSelect, ConnectionPanel, InputForm, OutputForm, PermissionsEditor, SaveDialog, SlotsEditor, StepForm, WirePrompt, newInput, newOutput, withPrompt,
} from "./RecipeEditor";
import type { InstalledModel, RecipeDoc, RecipeInfo, RecipeSource, Run, StepRecord, ToolInfo } from "../types";

/* Flow View: the recipe drawn as connected nodes, with the current run's state painted on top.
   Edit mode swaps the run overlay for a draft of the recipe: click a node to change it, drag
   between handles to wire it, and save the draft as your own recipe. */

interface Props {
  recipe: RecipeInfo;
  /** The run shown on the graph, if it is a run of this recipe. */
  run: Run | null;
  records: StepRecord[];
  inProgress: boolean;
  models: InstalledModel[];
  pins: Record<string, string>;
  onPin: (slot: string, modelId: string | null) => void;
  /** A recipe was saved from the editor; the app should reload its list and show it. */
  onSaved: (recipe: RecipeInfo) => void;
  /** Offer the editor. Off while watching a run, where the map is for reading. */
  editable?: boolean;
  /** Open straight into edit mode (a recipe just created). */
  startInEdit?: boolean;
}

interface EditState {
  sourceId: string;
  source: RecipeSource;
  doc: RecipeDoc;
  files: Record<string, string>;
  problems: string[];
  dirty: boolean;
}

/* Handles: steps feed steps top to bottom ("b" → "t"); inputs and results attach on the sides ("r" → "l"). */
function StepHandles() {
  return (
    <>
      <Handle type="target" position={Position.Top} id="t" />
      <Handle type="target" position={Position.Left} id="l" />
      <Handle type="source" position={Position.Bottom} id="b" />
      <Handle type="source" position={Position.Right} id="r" />
    </>
  );
}

function StepNode({ data, selected }: NodeProps<FlowNode>) {
  const elapsed = useElapsed(data.status === "running" ? data.startedAt : null);
  const statusText = data.status === "running" && elapsed != null ? `${data.statusText} ${fmtElapsed(elapsed)}` : data.statusText;
  return (
    <div className={`flow-node step kind-${data.kind ?? ""} ${data.status} ${data.provider === "cloud" ? "cloud" : ""} ${selected ? "selected" : ""}`} title="Click for details">
      <StepHandles />
      <Glyph name={stepGlyph(data.kind ?? "")} />
      <div className="flow-kind">{data.provider === "cloud" && <Glyph name="cloud" />}{data.kindLabel}</div>
      <div className="flow-title">{data.title}</div>
      {data.condition
        ? <div className="flow-cond" title={data.subtitle}>{data.condition}{data.subtitle ? <span className="muted"> · {data.subtitle}</span> : null}</div>
        : data.subtitle && <div className="flow-sub mono">{data.subtitle}</div>}
      {statusText && <div className={`flow-status ${data.status}`}>{statusText}</div>}
    </div>
  );
}

function IoNode({ data, selected }: NodeProps<FlowNode>) {
  return (
    <div className={`flow-node io ${data.role} ${data.status} ${selected ? "selected" : ""}`}>
      {data.role === "output" && <Handle type="target" position={Position.Left} id="l" />}
      <Glyph name={data.role === "input" ? inputGlyph(data.ioType ?? "") : outputGlyph(data.ioType ?? "")} />
      <div className="flow-kind">{data.kindLabel}</div>
      <div className="flow-title">{data.title}</div>
      {data.subtitle && <div className="flow-sub">{data.subtitle}</div>}
      {data.statusText && <div className={`flow-status ${data.status}`}>{data.statusText}</div>}
      {data.role === "input" && <Handle type="source" position={Position.Right} id="r" />}
    </div>
  );
}

function GroupNode({ data, selected }: NodeProps<FlowNode>) {
  return (
    <div className={`flow-node group ${data.status} ${selected ? "selected" : ""}`}>
      <StepHandles />
      <div className="flow-group-head">
        <div className="flow-title"><span className="flow-kind">{data.kindLabel}</span> {data.title}</div>
        <div className="flow-sub">{data.subtitle}{data.statusText && <> · <span className={`flow-status ${data.status}`}>{data.statusText}</span></>}</div>
      </div>
    </div>
  );
}

/** A path through the given points with rounded corners. */
function roundedPath(pts: [number, number][], r = 8): string {
  let d = `M ${pts[0][0]} ${pts[0][1]}`;
  for (let i = 1; i < pts.length - 1; i++) {
    const [px, py] = pts[i - 1];
    const [cx, cy] = pts[i];
    const [nx, ny] = pts[i + 1];
    const d1 = Math.hypot(cx - px, cy - py);
    const d2 = Math.hypot(nx - cx, ny - cy);
    if (d1 === 0 || d2 === 0) continue;
    const r1 = Math.min(r, d1 / 2);
    const r2 = Math.min(r, d2 / 2);
    d += ` L ${cx - ((cx - px) / d1) * r1} ${cy - ((cy - py) / d1) * r1} Q ${cx} ${cy} ${cx + ((nx - cx) / d2) * r2} ${cy + ((ny - cy) / d2) * r2}`;
  }
  const [lx, ly] = pts[pts.length - 1];
  return `${d} L ${lx} ${ly}`;
}

/* A result line that leaves a step from the bottom, runs along a bus under the row, then turns
   up or down into the result on the right. Keeps lines from crossing sibling steps. */
function BusEdge({ id, sourceX, sourceY, targetX, targetY, data, markerEnd, style }: EdgeProps) {
  const viaY = typeof data?.viaY === "number" ? data.viaY : sourceY + 24;
  const bendX = targetX - 28;
  const path = roundedPath([[sourceX, sourceY], [sourceX, viaY], [bendX, viaY], [bendX, targetY], [targetX, targetY]]);
  return <BaseEdge id={id} path={path} markerEnd={markerEnd} style={style} />;
}

const PAD = 16;

const nodeTypes: NodeTypes = { step: StepNode, io: IoNode, group: GroupNode };
const edgeTypes: EdgeTypes = { bus: BusEdge };

export default function FlowView({ recipe, run, records, inProgress, models, pins, onPin, onSaved, editable = true, startInEdit = false }: Props) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedEdge, setSelectedEdge] = useState<Edge | null>(null);
  const [edit, setEdit] = useState<EditState | null>(null);
  const [tools, setTools] = useState<ToolInfo[]>([]);
  const [wire, setWire] = useState<{ source: string; target: string } | null>(null);
  const [panel, setPanel] = useState<"node" | "slots" | "permissions" | "save">("node");
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [canvasW, setCanvasW] = useState(900);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    // A width of 0 means the canvas is hidden for the moment, not that it is that narrow.
    const measure = () => {
      const w = el.getBoundingClientRect().width;
      if (w > 0) setCanvasW(w);
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    window.addEventListener("resize", measure);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, []);

  // Leaving the recipe leaves the editor too; a recipe just made opens already in edit mode.
  useEffect(() => {
    setEdit(null); setSelectedId(null); setSelectedEdge(null); setWire(null); setPanel("node");
    if (startInEdit) startEditing();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recipe.id]);

  // Ask the runtime what is wrong with the draft, a moment after each change.
  useEffect(() => {
    if (!edit) return;
    const { doc, files } = edit;
    const t = setTimeout(() => {
      api.checkRecipeDraft({ yaml: doc, files })
        .then((problems) => setEdit((e) => (e && e.doc === doc && e.files === files ? { ...e, problems } : e)))
        .catch((err) => setEdit((e) => (e ? { ...e, problems: [String(err)] } : e)));
    }, 350);
    return () => clearTimeout(t);
  }, [edit?.doc, edit?.files]);



  async function startEditing() {
    setLoadError(null);
    try {
      const [r, t] = await Promise.all([api.readRecipeDraft(recipe.id), api.listTools()]);
      setTools(t);
      setEdit({ sourceId: r.id, source: r.source, doc: r.draft.yaml, files: r.draft.files, problems: [], dirty: false });
      setSelectedId(null);
      setSelectedEdge(null);
      setPanel("node");
    } catch (e) {
      setLoadError(String(e));
    }
  }

  const change = useCallback((doc: RecipeDoc, files?: Record<string, string>) => {
    setEdit((e) => (e ? { ...e, doc, files: files ?? e.files, dirty: true } : e));
  }, []);

  function select(nodeId: string | null) {
    setSelectedId(nodeId);
    setSelectedEdge(null);
    setWire(null);
    setPanel("node");
  }

  function stopEditing() {
    setEdit(null);
    setConfirmDiscard(false);
    select(null);
  }

  async function save(id: string, name: string, description: string, icon: string | null) {
    if (!edit) return;
    setSaving(true);
    setSaveError(null);
    try {
      const doc = mutate(edit.doc, (d) => {
        d.id = id;
        d.name = name;
        if (description) d.description = description; else delete d.description;
        if (icon) d.icon = icon; else delete d.icon;
        // A saved copy is yours, whatever it was copied from.
        if (Array.isArray(d.tags)) d.tags = (d.tags as string[]).filter((t) => t !== "built-in");
        if (edit.source === "builtin" && id !== edit.sourceId) delete d.author;
      });
      const r = await api.saveRecipeDraft({ yaml: doc, files: edit.files });
      stopEditing();
      onSaved(r.recipe);
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  }

  function onConnect(c: Connection) {
    if (!edit || !c.source || !c.target || c.source === c.target) return;
    if (c.target.startsWith("out:")) {
      change(connect(edit.doc, c.source, c.target));
      select(c.target);
    } else if (stepPathOf(c.target)) {
      setSelectedId(c.target);
      setSelectedEdge(null);
      setPanel("node");
      setWire({ source: c.source, target: c.target });
    }
  }

  const modelName = (id: string) => models.find((m) => m.id === id)?.name ?? id.replace(/^[a-z]+:/, "");
  const modelFor = (slot: string) => {
    const id = run?.resolutions?.[slot]?.modelId ?? pins[slot];
    return id ? modelName(id) : null;
  };
  // Every provider is Ollama on this Mac today; anything else would be a cloud model and marked amber.
  const providerFor = (slot: string): "local" | "cloud" | null => {
    const id = run?.resolutions?.[slot]?.modelId ?? pins[slot];
    if (!id) return "local";
    return id.startsWith("ollama:") ? "local" : "cloud";
  };

  const shownRecipe = edit ? docToInfo(edit.doc, recipe) : recipe;
  const shownRun = edit ? null : run;
  const shownRecords = edit ? [] : records;
  const shownProgress = edit ? false : inProgress;

  const { nodes, edges, width, height, lanes } = useMemo(
    () => {
      const g = buildGraph({ recipe: shownRecipe, run: shownRun, records: shownRecords, inProgress: shownProgress, modelFor, providerFor, selectedId });
      if (selectedEdge) g.edges = g.edges.map((e) => (e.id === selectedEdge.id ? { ...e, selected: true, className: `${e.className} selected` } : e));
      return g;
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [shownRecipe, shownRun, shownRecords, shownProgress, pins, models, selectedId, selectedEdge],
  );

  const labelOf = (nodeId: string) => nodes.find((n) => n.id === nodeId)?.data.title ?? nodeId;

  // The drawing is shown whole, at full size when it fits the canvas width, smaller only if it must.
  // The page scrolls; the canvas does not pan or zoom.
  const zoom = Math.min(1, Math.max(0.1, (canvasW - PAD * 2) / Math.max(1, width)));
  const canvasH = Math.ceil(height * zoom + PAD * 2);
  const viewport = { x: PAD + (canvasW - PAD * 2 - width * zoom) / 2, y: PAD, zoom };

  const selectedPath = selectedId ? stepPathOf(selectedId) : null;
  const selectedMap = edit && selectedPath && !selectedPath.includes("/") && edit.doc.steps.find((s) => s.id === selectedPath && s.kind === "map");
  const hasPanel = Boolean(selectedId || selectedEdge || wire || panel !== "node");

  return (
    <section className={`panel flow-panel ${edit ? "editing" : ""}`}>
      <div className="row space" style={{ margin: "0 0 8px" }}>
        <h2 style={{ margin: 0 }}>
          Recipe card <span className="muted">{edit ? "(editing a draft of the recipe)" : "(the recipe written out as a map: inputs → steps → results)"}</span>
        </h2>
        {edit ? (
          <span className="muted">{edit.dirty ? "unsaved changes" : "no changes yet"}</span>
        ) : (
          <span className="row" style={{ margin: 0 }}>
            <span className="muted">{recipe.name}{run ? ` · run ${run.id.slice(-8)}` : " · no run yet"}</span>
            {editable && <button onClick={startEditing}>Edit this recipe</button>}
          </span>
        )}
      </div>
      {loadError && <pre className="error">{loadError}</pre>}

      {edit && (
        <div className="flow-toolbar">
          <AddStepSelect label={selectedMap ? `Add step inside “${selectedMap.name ?? selectedMap.id}”` : "Add step"} onPick={(kind) => {
            const r = addStep(edit.doc, kind, selectedMap ? selectedMap.id : undefined);
            change(r.doc, withPrompt(edit.files, r));
            select(`step:${r.path}`);
          }} />
          <button onClick={() => { const { name, spec } = newInput(edit.doc); change(mutate(edit.doc, (d) => { (d.inputs ??= {})[name] = spec; })); select(`in:${name}`); }}>Add input</button>
          <button onClick={() => { const { name, spec } = newOutput(edit.doc); change(mutate(edit.doc, (d) => { d.outputs[name] = spec; })); select(`out:${name}`); }}>Add result</button>
          <button onClick={() => { setPanel("slots"); setSelectedId(null); setSelectedEdge(null); setWire(null); }}>Model slots</button>
          <button onClick={() => { setPanel("permissions"); setSelectedId(null); setSelectedEdge(null); setWire(null); }}>Permissions</button>
          <span className="flow-toolbar-gap" />
          <button className="primary" style={{ marginTop: 0 }} disabled={edit.problems.length > 0} title={edit.problems.length ? "Fix the problems first" : ""}
            onClick={() => { setPanel("save"); setSaveError(null); setSelectedId(null); setSelectedEdge(null); setWire(null); }}>Save…</button>
          {confirmDiscard
            ? <><button className="danger" onClick={stopEditing}>Yes, discard</button><button onClick={() => setConfirmDiscard(false)}>Keep editing</button></>
            : <button onClick={() => (edit.dirty ? setConfirmDiscard(true) : stopEditing())}>{edit.dirty ? "Discard…" : "Done"}</button>}
        </div>
      )}
      {edit && edit.problems.length > 0 && (
        <div className="tty flow-problems" role="log">
          {edit.problems.map((p, i) => <div key={i} className="red"><span className="mark">✗ </span>{p}</div>)}
        </div>
      )}

      <div className={`flow-layout ${hasPanel ? "with-details" : ""}`}>
        <div className="flow-canvas" ref={wrapRef} style={{ height: canvasH }}>
          {lanes.in && (
            <div className="lane-band in" style={{ left: viewport.x + lanes.in.x * zoom, width: lanes.in.w * zoom }}>
              <span className="lane-label"><Glyph name="in" /> In · what goes in</span>
            </div>
          )}
          {lanes.out && (
            <div className="lane-band out" style={{ left: viewport.x + lanes.out.x * zoom, width: lanes.out.w * zoom }}>
              <span className="lane-label"><Glyph name="out" /> Out · what you get</span>
            </div>
          )}
          <ReactFlow
            nodes={nodes}
            edges={edges}
            nodeTypes={nodeTypes}
            edgeTypes={edgeTypes}
            viewport={viewport}
            onViewportChange={() => {}}
            minZoom={0.1}
            maxZoom={1}
            nodesDraggable={false}
            nodesConnectable={Boolean(edit)}
            elementsSelectable
            selectNodesOnDrag={false}
            panOnDrag={false}
            panOnScroll={false}
            zoomOnScroll={false}
            zoomOnPinch={false}
            zoomOnDoubleClick={false}
            preventScrolling={false}
            deleteKeyCode={null}
            colorMode={document.documentElement.dataset.theme === "paper" ? "light" : "dark"}
            onNodeClick={(_, n) => select(n.id)}
            onEdgeClick={(_, e) => { if (edit) { setSelectedEdge(e); setSelectedId(null); setWire(null); setPanel("node"); } }}
            onPaneClick={() => select(null)}
            onConnect={onConnect}
          >
            <Background gap={24} />
          </ReactFlow>
        </div>

        {!edit && selectedId && (
          <StepDetails
            recipe={recipe} run={run} records={records} models={models} pins={pins} onPin={onPin}
            selectedId={selectedId} onClose={() => select(null)}
          />
        )}
        {!edit && !selectedId && (
          <p className="muted flow-hint">{inProgress
            ? "Click any step for details, even while it runs: the model it is using, how long it has been going, and why an AI step can take a while."
            : "Click a step to see which model it uses, what it reads, and what it produced. A dashed box is a for-each: its inner steps run once per item."}</p>
        )}

        {edit && hasPanel && (
          <aside className="flow-details">
            {panel === "save" && (
              <SaveDialog doc={edit.doc} source={edit.source} sourceId={edit.sourceId} busy={saving} error={saveError} onSave={save} onCancel={() => setPanel("node")} />
            )}
            {panel === "slots" && <SlotsEditor doc={edit.doc} onChange={change} onClose={() => setPanel("node")} />}
            {panel === "permissions" && <PermissionsEditor doc={edit.doc} onChange={change} onClose={() => setPanel("node")} />}
            {panel === "node" && wire && (
              <WirePrompt doc={edit.doc} source={wire.source} target={wire.target}
                onConnect={(key) => { change(connect(edit.doc, wire.source, wire.target, key)); setWire(null); }}
                onCancel={() => setWire(null)} />
            )}
            {panel === "node" && !wire && selectedEdge && (
              <ConnectionPanel doc={edit.doc} edge={selectedEdge} labelOf={labelOf}
                onDisconnect={() => { change(disconnect(edit.doc, selectedEdge.source, selectedEdge.target)); setSelectedEdge(null); }}
                onClose={() => setSelectedEdge(null)} />
            )}
            {panel === "node" && !wire && !selectedEdge && selectedId && (
              <>
                <button className="link flow-close" onClick={() => select(null)}>close</button>
                {selectedId.startsWith("in:") && (
                  <InputForm doc={edit.doc} name={selectedId.slice(3)} onChange={change} onSelect={select}
                    onRemove={() => { change(mutate(edit.doc, (d) => { delete d.inputs?.[selectedId.slice(3)]; })); select(null); }} />
                )}
                {selectedId.startsWith("out:") && (
                  <OutputForm doc={edit.doc} name={selectedId.slice(4)} onChange={change} onSelect={select}
                    onRemove={() => { change(mutate(edit.doc, (d) => { delete d.outputs[selectedId.slice(4)]; })); select(null); }} />
                )}
                {selectedPath && (
                  <StepForm doc={edit.doc} files={edit.files} path={selectedPath} tools={tools} onChange={change}
                    onSelect={(p) => select(`step:${p}`)}
                    onRemove={() => { change(removeStep(edit.doc, selectedPath)); select(null); }} />
                )}
              </>
            )}
          </aside>
        )}
        {edit && !hasPanel && (
          <p className="muted flow-hint">Click a step, input, or result to change it. Drag from a handle on one node to another to connect them, or click a line to cut it. Add steps with the menu above. Nothing is saved until you press Save.</p>
        )}
      </div>
    </section>
  );
}
