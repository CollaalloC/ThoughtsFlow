import { ArrowLeft, GitCompareArrows, Map, MessageSquareText, Settings2 } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import type { DecisionRunOption } from "../features/decision";
import { FocusWorkspace } from "../features/conversation";
import { resolveContextBranchId } from "../features/context-tree";
import { createDesktopBridge, type DesktopBridge } from "../platform/desktop-bridge";
import type {
  ContextTreeProjection,
  RouteProjection,
  WorkspaceDetail,
} from "../shared/contracts";
import { Brand, ErrorState, LoadingState } from "../shared/ui";
import "../shared/tokens/index.css";
import "../shared/ui/styles.css";
import "./app.css";

type AppView = "focus" | "route" | "decision" | "settings";

const DecisionWorkspace = lazy(() =>
  import("../features/decision").then((module) => ({ default: module.DecisionWorkspace })),
);
const RouteMap = lazy(() =>
  import("../features/route-map").then((module) => ({ default: module.RouteMap })),
);
const ProviderSettings = lazy(() =>
  import("../features/settings").then((module) => ({ default: module.ProviderSettings })),
);

export interface AppProps {
  bridge?: DesktopBridge;
}

function runLabel(answerIndex: number) {
  return `回答 ${String.fromCharCode(65 + (answerIndex % 26))}`;
}

function decisionRuns(detail: WorkspaceDetail): DecisionRunOption[] {
  return detail.turns.flatMap((turn) =>
    turn.runs.map((run, index) => ({
      runId: run.id,
      label: `${turn.title?.trim() || turn.prompt.trim().slice(0, 36)} · ${runLabel(index)}`,
      model: run.model,
      status: run.status,
    })),
  );
}

export function App({ bridge: providedBridge }: AppProps) {
  const bridgeRef = useRef<DesktopBridge | null>(null);
  if (!bridgeRef.current) bridgeRef.current = providedBridge ?? createDesktopBridge();
  const bridge = bridgeRef.current;

  const [view, setView] = useState<AppView>("focus");
  const [workspaceId, setWorkspaceId] = useState<string>();
  const [detail, setDetail] = useState<WorkspaceDetail | null>(null);
  const [projection, setProjection] = useState<RouteProjection | null>(null);
  const [contextTree, setContextTree] = useState<ContextTreeProjection | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
  const [settingsProviderProfileId, setSettingsProviderProfileId] = useState<string>();

  useEffect(() => {
    let active = true;
    bridge
      .listWorkspaces()
      .then((workspaces) => {
        if (!active || workspaceId) return;
        setWorkspaceId(workspaces.find((workspace) => !workspace.archived)?.id);
      })
      .catch(() => {
        // FocusWorkspace owns the primary initialization error; shell navigation remains usable.
      });
    return () => {
      active = false;
    };
  }, [bridge, workspaceId]);

  useEffect(() => {
    if (view !== "route" && view !== "decision") return;
    if (!workspaceId) {
      setDetail(null);
      setProjection(null);
      setContextTree(null);
      setLoading(false);
      setError("当前没有可打开的本地工作区。");
      return;
    }

    let active = true;
    setLoading(true);
    setError(null);

    const request =
      view === "route"
        ? Promise.all([
            bridge.openWorkspace(workspaceId),
            bridge.getRouteProjection({ workspaceId }),
            bridge.getContextTree({ workspaceId }),
          ]).then(([nextDetail, nextProjection, nextContextTree]) => ({
            detail: nextDetail,
            projection: nextProjection,
            contextTree: nextContextTree,
          }))
        : bridge.openWorkspace(workspaceId).then((nextDetail) => ({
            detail: nextDetail,
            projection: null,
            contextTree: null,
          }));

    request
      .then((result) => {
        if (!active) return;
        setDetail(result.detail);
        setProjection(result.projection);
        setContextTree(result.contextTree);
      })
      .catch((reason: unknown) => {
        if (!active) return;
        setDetail(null);
        setProjection(null);
        setContextTree(null);
        setError(reason instanceof Error ? reason.message : "无法读取工作区视图。");
      })
      .finally(() => {
        if (active) setLoading(false);
      });

    return () => {
      active = false;
    };
  }, [bridge, reloadKey, view, workspaceId]);

  const runs = useMemo(() => (detail ? decisionRuns(detail) : []), [detail]);
  const routeSelectedRunIds = useMemo(() => {
    const selected: Record<string, string> = {};
    contextTree?.nodes
      .filter((node) => node.isOnActivePath)
      .forEach((node) => {
        selected[node.turnId] = node.runId;
      });
    return selected;
  }, [contextTree]);

  const openOrdinaryView = (nextView: Exclude<AppView, "settings">) => {
    setSettingsProviderProfileId(undefined);
    setView(nextView);
  };

  const openSettings = (providerProfileId?: string) => {
    setError(null);
    setSettingsProviderProfileId(providerProfileId);
    setView("settings");
  };

  const openRoute = (nextWorkspaceId: string, _runId?: string) => {
    setWorkspaceId(nextWorkspaceId);
    setProjection(null);
    setError(null);
    openOrdinaryView("route");
  };

  const openDecisions = (nextWorkspaceId: string) => {
    setWorkspaceId(nextWorkspaceId);
    setError(null);
    openOrdinaryView("decision");
  };

  const openWorkspaceView = (nextView: "route" | "decision") => {
    setError(null);
    if (nextView === "route") setProjection(null);
    openOrdinaryView(nextView);
  };

  const persistViewState = (change: { turnId: string; x: number; y: number }) => {
    if (!workspaceId) return;
    void bridge
      .updateViewState({ workspaceId, ...change, collapsed: false })
      .catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : "无法保存路线图位置。");
      });
  };

  const selectRouteRun = async (runId: string, returnToFocus = false) => {
    if (!workspaceId || !contextTree) return;
    const node = contextTree.nodes.find((item) => item.runId === runId);
    const branchId = node
      ? resolveContextBranchId(contextTree, node.branchIds)
      : null;
    setError(null);
    try {
      const cursor = await bridge.setActiveContext({
        workspaceId,
        runId,
        branchId,
        expectedCursorVersion: contextTree.cursor.version,
        expectedDraftVersion: contextTree.draftVersion,
      });
      setContextTree((current) => current ? {
        ...current,
        cursor,
      } : current);
      if (returnToFocus) {
        openOrdinaryView("focus");
      } else {
        setProjection(null);
        setReloadKey((current) => current + 1);
      }
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "无法切换 Context 位置。");
      try {
        setContextTree(await bridge.getContextTree({ workspaceId }));
      } catch {
        // The structured cursor failure remains the most useful error.
      }
    }
  };

  return (
    <div className="app-shell" data-view={view}>
      <header className="app-shell__bar">
        <Brand />
        <nav aria-label="工作面" className="app-shell__navigation">
          <button
            aria-current={view === "focus" ? "page" : undefined}
            onClick={() => openOrdinaryView("focus")}
            type="button"
          >
            <MessageSquareText aria-hidden="true" size={14} /> Focus
          </button>
          <button
            aria-current={view === "route" ? "page" : undefined}
            disabled={!workspaceId}
            onClick={() => openWorkspaceView("route")}
            type="button"
          >
            <Map aria-hidden="true" size={14} /> 路线图
          </button>
          <button
            aria-current={view === "decision" ? "page" : undefined}
            disabled={!workspaceId}
            onClick={() => openWorkspaceView("decision")}
            type="button"
          >
            <GitCompareArrows aria-hidden="true" size={14} /> 决策
          </button>
          <button
            aria-current={view === "settings" ? "page" : undefined}
            onClick={() => openSettings()}
            type="button"
          >
            <Settings2 aria-hidden="true" size={14} /> Provider 设置
          </button>
        </nav>
      </header>

      <main className="app-shell__content">
        {view === "focus" ? (
          <FocusWorkspace
            bridge={bridge}
            initialWorkspaceId={workspaceId}
            onOpenDecisions={openDecisions}
            onOpenRouteMap={openRoute}
            onOpenSettings={openSettings}
          />
        ) : (
          <section className="app-shell__workspace-view" aria-label={`${view} 工作面`}>
            <header className="app-shell__view-header">
              <button
                aria-label="返回 Focus"
                onClick={() => openOrdinaryView("focus")}
                type="button"
              >
                <ArrowLeft aria-hidden="true" size={15} /> 返回 Focus
              </button>
              <div>
                <span>{view === "route" ? "ROUTE MAP" : view === "decision" ? "DECISION" : "PROVIDERS"}</span>
                {detail?.workspace.name ? <strong>{detail.workspace.name}</strong> : null}
              </div>
            </header>

            <Suspense fallback={<LoadingState label="正在加载工作面" />}>
              {view === "settings" ? (
                <ProviderSettings
                  bridge={bridge}
                  initialProviderProfileId={settingsProviderProfileId}
                />
              ) : null}

              {view !== "settings" && loading ? (
                <LoadingState label={view === "route" ? "正在读取路线投影" : "正在读取决策工作区"} />
              ) : null}

              {view !== "settings" && error ? (
                <div className="app-shell__error">
                  <ErrorState message={error} />
                  <button onClick={() => setReloadKey((value) => value + 1)} type="button">
                    重试加载
                  </button>
                </div>
              ) : null}

              {view === "route" && !loading && !error && projection ? (
                <RouteMap
                selectedRunIds={routeSelectedRunIds}
                onCreateBranch={(parentRunId) => {
                  void selectRouteRun(parentRunId, true);
                }}
                onSelectRun={(runId) => void selectRouteRun(runId)}
                onSelectTurn={(turnId) => {
                  const node = projection.nodes.find((item) => item.turnId === turnId);
                  const activeRunId = contextTree?.nodes.find(
                    (item) => item.turnId === turnId && item.isOnActivePath,
                  )?.runId;
                  const runId = activeRunId
                    ?? node?.runs.find((run) => run.status === "completed")?.runId;
                  if (runId) void selectRouteRun(runId);
                }}
                onViewStateChange={persistViewState}
                projection={projection}
                />
              ) : null}

              {view === "decision" && !loading && !error && detail ? (
                <DecisionWorkspace
                existingMarks={detail.decisionMarks}
                onCompare={(input) => bridge.compareRuns(input)}
                onExport={(input) => bridge.exportDecisionPacket(input)}
                onMarkDecision={(input) => bridge.markDecision(input)}
                runs={runs}
                workspaceId={detail.workspace.id}
                />
              ) : null}
            </Suspense>
          </section>
        )}
      </main>
    </div>
  );
}
