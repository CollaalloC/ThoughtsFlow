import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  type Edge,
  type Node,
  type NodeProps,
  type OnNodeDrag,
} from "@xyflow/react";
import { GitBranch, LocateFixed, Route } from "lucide-react";
import { useMemo } from "react";
import "@xyflow/react/dist/style.css";
import "./route-map.css";

export type RouteRunStatus =
  | "pending"
  | "connecting"
  | "streaming"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted";

export interface RouteRunView {
  runId: string;
  label: string;
  model?: string;
  status: RouteRunStatus | string;
  canBranch: boolean;
}

export interface RouteNodeView {
  id: string;
  turnId: string;
  runId?: string;
  parentRunId?: string;
  title: string;
  summary: string;
  status: RouteRunStatus | string;
  decision?: "accepted" | "rejected" | "to-verify";
  x: number;
  y: number;
  isCurrent: boolean;
  isOnCurrentLineage?: boolean;
  runs?: RouteRunView[];
}

export interface RouteEdgeView {
  id: string;
  sourceRunId: string;
  targetTurnId: string;
  isOnCurrentLineage?: boolean;
}

export interface RouteProjection {
  workspaceId: string;
  nodes: RouteNodeView[];
  edges: RouteEdgeView[];
}

export interface RouteMapProps {
  projection: RouteProjection;
  selectedRunIds?: Record<string, string>;
  /** @deprecated Prefer selectedRunIds so every Turn on the active path is exact. */
  activeRunId?: string | null;
  onSelectTurn?: (turnId: string) => void;
  onSelectRun: (runId: string) => void;
  onCreateBranch: (parentRunId: string) => void;
  onViewStateChange?: (change: { turnId: string; x: number; y: number }) => void;
}

type RouteNodeData = Record<string, unknown> & {
  view: RouteNodeView;
  runs: RouteRunView[];
  selectedRunId?: string | null;
  onSelectTurn?: (turnId: string) => void;
  onSelectRun: (runId: string) => void;
  onCreateBranch: (parentRunId: string) => void;
};

const statusLabels: Record<string, string> = {
  queued: "等待中",
  connecting: "连接中",
  streaming: "生成中",
  completed: "已完成",
  failed: "失败",
  cancelled: "已取消",
  interrupted: "已中断",
};

const decisionLabels = {
  accepted: "已采纳",
  rejected: "已否决",
  "to-verify": "待验证",
} as const;

function normalizeRuns(view: RouteNodeView): RouteRunView[] {
  if (view.runs?.length) return view.runs;
  if (!view.runId) return [];

  return [
    {
      runId: view.runId,
      label: "回答",
      status: view.status,
      canBranch: view.status === "completed",
    },
  ];
}

function RouteTurnNode({ data }: NodeProps<Node<RouteNodeData>>) {
  const { view, runs, selectedRunId, onSelectTurn, onSelectRun, onCreateBranch } = data;

  return (
    <article
      aria-label={view.title}
      className="route-turn-node"
      data-current={String(view.isCurrent)}
      data-lineage={String(Boolean(view.isOnCurrentLineage ?? view.isCurrent))}
      data-decision={view.decision}
      onClick={() => onSelectTurn?.(view.turnId)}
    >
      <Handle id="incoming" type="target" position={Position.Left} />
      <header className="route-turn-node__header">
        <span className="route-turn-node__eyebrow">
          {view.isCurrent ? "CURRENT" : "QUESTION"}
        </span>
        <span className={`route-turn-node__status route-turn-node__status--${view.status}`}>
          {statusLabels[view.status] ?? view.status}
        </span>
      </header>
      <h3>{view.title}</h3>
      <p>{view.summary}</p>
      {view.decision ? (
        <span className={`route-turn-node__decision route-turn-node__decision--${view.decision}`}>
          {decisionLabels[view.decision]}
        </span>
      ) : null}
      {runs.length ? (
        <div className="route-turn-node__runs" aria-label={`${view.title}的回答版本`}>
          {runs.map((run) => (
            <div className="route-run-port" key={run.runId}>
              <button
                aria-label={`选择${view.title}的${run.label}`}
                aria-pressed={run.runId === selectedRunId}
                className={`route-run-port__select nodrag ${run.runId === selectedRunId ? "is-active" : ""}`}
                data-run-id={run.runId}
                onClick={(event) => {
                  event.stopPropagation();
                  onSelectRun(run.runId);
                }}
                type="button"
              >
                <span>{run.label}</span>
                {run.model ? <code>{run.model}</code> : null}
              </button>
              <button
                aria-label={`从${view.title}的${run.label}创建分支`}
                className="route-run-port__branch nodrag"
                disabled={!run.canBranch}
                onClick={(event) => {
                  event.stopPropagation();
                  onCreateBranch(run.runId);
                }}
                title={run.canBranch ? `从精确 Run ${run.runId} 分支` : "此回答暂不能分支"}
                type="button"
              >
                <GitBranch aria-hidden="true" size={13} />
              </button>
              <Handle
                id={run.runId}
                type="source"
                position={Position.Right}
                style={{ top: "50%" }}
              />
            </div>
          ))}
        </div>
      ) : (
        <span className="route-turn-node__empty">暂无回答</span>
      )}
    </article>
  );
}

const nodeTypes = { routeTurn: RouteTurnNode };

export function RouteMap({
  projection,
  selectedRunIds,
  activeRunId,
  onSelectTurn,
  onSelectRun,
  onCreateBranch,
  onViewStateChange,
}: RouteMapProps) {
  const runOwners = useMemo(() => {
    const owners = new Map<string, string>();
    projection.nodes.forEach((view) => {
      normalizeRuns(view).forEach((run) => owners.set(run.runId, view.id));
    });
    return owners;
  }, [projection.nodes]);

  const turnNodes = useMemo<Node<RouteNodeData>[]>(
    () =>
      projection.nodes.map((view) => ({
        id: view.id,
        type: "routeTurn",
        position: { x: view.x, y: view.y },
        initialWidth: 250,
        initialHeight: 142 + normalizeRuns(view).length * 36,
        data: {
          view,
          runs: normalizeRuns(view),
          selectedRunId: selectedRunIds?.[view.turnId]
            ?? (normalizeRuns(view).some((run) => run.runId === activeRunId)
              ? activeRunId
              : null),
          onSelectTurn,
          onSelectRun,
          onCreateBranch,
        },
        className: view.isOnCurrentLineage ?? view.isCurrent ? "is-lineage" : undefined,
        ariaLabel: view.title,
      })),
    [
      activeRunId,
      onCreateBranch,
      onSelectRun,
      onSelectTurn,
      projection.nodes,
      selectedRunIds,
    ],
  );

  const turnNodeIds = useMemo(
    () => new Map(projection.nodes.map((node) => [node.turnId, node.id])),
    [projection.nodes],
  );

  const routeEdges = useMemo<Edge[]>(
    () =>
      projection.edges.flatMap((view) => {
        const source = runOwners.get(view.sourceRunId);
        const target = turnNodeIds.get(view.targetTurnId);
        if (!source || !target) return [];

        const isLineage = Boolean(view.isOnCurrentLineage);
        return [
          {
            id: view.id,
            source,
            target,
            sourceHandle: view.sourceRunId,
            targetHandle: "incoming",
            className: isLineage ? "route-edge--lineage" : "route-edge--adjacent",
            animated: false,
            markerEnd: { type: MarkerType.ArrowClosed },
            style: {
              stroke: isLineage ? "var(--route-accent)" : "var(--route-line)",
              strokeWidth: isLineage ? 2 : 1,
            },
          },
        ];
      }),
    [projection.edges, runOwners, turnNodeIds],
  );

  const lineageCount = projection.nodes.filter(
    (node) => node.isOnCurrentLineage ?? node.isCurrent,
  ).length;

  const handleNodeDragStop: OnNodeDrag<Node<RouteNodeData>> = (_, node) => {
    const view = node.data.view;
    onViewStateChange?.({ turnId: view.turnId, x: node.position.x, y: node.position.y });
  };

  return (
    <section className="route-map" aria-label="对话路线图">
      <header className="route-map__header">
        <div>
          <span className="route-map__eyebrow">
            <Route aria-hidden="true" size={14} /> ROUTE MAP
          </span>
          <h2>当前路线 · {lineageCount} 个问题</h2>
          <p>位置只影响视图；连线始终来自精确回答版本。</p>
        </div>
        <span className="route-map__locate-hint">
          <LocateFixed aria-hidden="true" size={14} /> 使用画布控制定位当前路线
        </span>
      </header>
      <div className="route-map__canvas">
        <ReactFlow
          colorMode="light"
          edges={routeEdges}
          fitView
          fitViewOptions={{ padding: 0.22, includeHiddenNodes: false }}
          minZoom={0.3}
          nodeTypes={nodeTypes}
          nodes={turnNodes}
          nodesConnectable={false}
          nodesFocusable
          onNodeDragStop={handleNodeDragStop}
          proOptions={{ hideAttribution: true }}
        >
          <Background color="var(--route-grid)" gap={24} variant={BackgroundVariant.Dots} />
          <Controls position="bottom-right" showInteractive={false} />
        </ReactFlow>
      </div>
    </section>
  );
}
