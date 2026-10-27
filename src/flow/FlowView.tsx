import { useEffect, useMemo, useRef, useState } from "react";
import { Background, Handle, Position, ReactFlow, type NodeProps, type NodeTypes } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { buildGraph, type FlowNode } from "./graph";
import { StepDetails } from "./StepDetails";
import type { InstalledModel, RecipeInfo, Run, StepRecord } from "../types";

/* Flow View: the recipe drawn as connected nodes, with the current run's state painted on top.
   Read-only in this version: you can inspect every step and pin a model, not rewire the graph. */

interface Props {
  recipe: RecipeInfo;
  /** The run shown on the graph, if it is a run of this recipe. */
  run: Run | null;
  records: StepRecord[];
  inProgress: boolean;
  models: InstalledModel[];
  pins: Record<string, string>;
  onPin: (slot: string, modelId: string | null) => void;
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
  return (
    <div className={`flow-node step ${data.status} ${selected ? "selected" : ""}`}>
      <StepHandles />
      <div className="flow-kind">{data.kindLabel}</div>
      <div className="flow-title">{data.title}</div>
      {data.subtitle && <div className="flow-sub mono">{data.subtitle}</div>}
      {data.condition && <div className="flow-cond">{data.condition}</div>}
      {data.statusText && <div className={`flow-status ${data.status}`}>{data.statusText}</div>}
    </div>
  );
}

function IoNode({ data, selected }: NodeProps<FlowNode>) {
  return (
    <div className={`flow-node io ${data.role} ${data.status} ${selected ? "selected" : ""}`}>
      {data.role === "output" && <Handle type="target" position={Position.Left} id="l" />}
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

const PAD = 16;

const nodeTypes: NodeTypes = { step: StepNode, io: IoNode, group: GroupNode };

export default function FlowView({ recipe, run, records, inProgress, models, pins, onPin }: Props) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [canvasW, setCanvasW] = useState(900);
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setCanvasW(e.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const modelName = (id: string) => models.find((m) => m.id === id)?.name ?? id.replace(/^[a-z]+:/, "");
  const modelFor = (slot: string) => {
    const id = run?.resolutions?.[slot]?.modelId ?? pins[slot];
    return id ? modelName(id) : null;
  };

  const { nodes, edges, width, height } = useMemo(
    () => buildGraph({ recipe, run, records, inProgress, modelFor, selectedId }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [recipe, run, records, inProgress, pins, models, selectedId],
  );

  // The drawing is shown whole, at full size when it fits the canvas width, smaller only if it must.
  // The page scrolls; the canvas does not pan or zoom.
  const zoom = Math.min(1, Math.max(0.1, (canvasW - PAD * 2) / Math.max(1, width)));
  const canvasH = Math.ceil(height * zoom + PAD * 2);
  const viewport = { x: PAD + (canvasW - PAD * 2 - width * zoom) / 2, y: PAD, zoom };

  return (
    <section className="panel flow-panel">
      <div className="row space" style={{ margin: "0 0 8px" }}>
        <h2 style={{ margin: 0 }}>Flow <span className="muted">(the recipe as a map: inputs → steps → results)</span></h2>
        <span className="muted">{recipe.name}{run ? ` · run ${run.id.slice(-8)}` : " · no run yet"}</span>
      </div>
      <div className={`flow-layout ${selectedId ? "with-details" : ""}`}>
        <div className="flow-canvas" ref={wrapRef} style={{ height: canvasH }}>
          <ReactFlow
            nodes={nodes}
            edges={edges}
            nodeTypes={nodeTypes}
            viewport={viewport}
            onViewportChange={() => {}}
            minZoom={0.1}
            maxZoom={1}
            nodesDraggable={false}
            nodesConnectable={false}
            elementsSelectable
            selectNodesOnDrag={false}
            panOnDrag={false}
            panOnScroll={false}
            zoomOnScroll={false}
            zoomOnPinch={false}
            zoomOnDoubleClick={false}
            preventScrolling={false}
            colorMode="system"
            onNodeClick={(_, n) => setSelectedId(n.id)}
            onPaneClick={() => setSelectedId(null)}
          >
            <Background gap={24} />
          </ReactFlow>
        </div>
        {selectedId ? (
          <StepDetails
            recipe={recipe} run={run} records={records} models={models} pins={pins} onPin={onPin}
            selectedId={selectedId} onClose={() => setSelectedId(null)}
          />
        ) : (
          <p className="muted flow-hint">Click a step to see which model it uses, what it reads, and what it produced. A dashed box is a for-each: its inner steps run once per item.</p>
        )}
      </div>
    </section>
  );
}
