import {
  Archive,
  ArrowUp,
  Check,
  ChevronRight,
  CirclePlus,
  Copy,
  FileText,
  GitBranch,
  LoaderCircle,
  PanelRight,
  Pin,
  RefreshCw,
  Route,
  Search,
  Send,
  Settings2,
  Square,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent, KeyboardEvent } from "react";
import { DesktopBridgeError, type DesktopBridge } from "../../platform/desktop-bridge";
import type {
  ContextPreview,
  ModelRun,
  ProviderAuthPlacement,
  ProviderProfile,
  ProviderStreamProtocol,
  RunEvent,
  Turn,
  WorkspaceDetail,
  WorkspaceSummary,
} from "../../shared/contracts";
import {
  ContextInspector,
  type ContextInspectorItem,
  type InspectorRun,
  type InspectorTab,
  type LockedSnapshot,
  type LockedSnapshotProviderMetadata,
} from "../context-inspector";
import {
  Brand,
  Button,
  ErrorState,
  FlowMark,
  LoadingState,
  LocalDataBadge,
  ProviderDestination,
  SafeMarkdown,
} from "../../shared/ui";
import "../../shared/tokens/index.css";
import "../../shared/ui/styles.css";
import "./focus-workspace.css";

type WorkspaceView = WorkspaceSummary;
type RunView = ModelRun;
type TurnView = Turn;
type WorkspaceDetailView = WorkspaceDetail;
type ContextPreviewView = ContextPreview;
type ProviderProfileView = ProviderProfile;
type RunEventView = RunEvent;

type FocusWorkspaceProps = {
  bridge: DesktopBridge;
  initialWorkspaceId?: string;
  initialRunId?: string;
  onOpenRouteMap?: (workspaceId: string, runId?: string) => void;
  onOpenDecisions?: (workspaceId: string) => void;
  onOpenSettings?: () => void;
};

type SelectedRuns = Record<string, string>;

type RetryAction = {
  label: string;
  execute: () => void;
};

function millis(value?: number | string | null) {
  if (!value) return 0;
  if (typeof value === "string") return new Date(value).getTime();
  return value < 1e12 ? value * 1000 : value;
}

function formatTime(value: number | string) {
  const date = new Date(millis(value));
  return Number.isNaN(date.getTime())
    ? ""
    : new Intl.DateTimeFormat("zh-CN", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(date);
}

function formatDuration(run: RunView) {
  const end = millis(run.completedAt) || Date.now();
  const start = millis(run.createdAt);
  if (!start) return "";
  const seconds = Math.max(0, Math.round((end - start) / 1000));
  return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

function formatUsage(usage?: Record<string, number> | null) {
  const total = usage?.totalTokens ?? (usage?.inputTokens ?? 0) + (usage?.outputTokens ?? 0);
  return total ? `${total.toLocaleString()} tok` : "usage 待返回";
}

function runLabel(index: number) {
  return `回答 ${String.fromCharCode(65 + (index % 26))}`;
}

function isLocalProfile(profile?: ProviderProfileView) {
  if (!profile) return false;
  try {
    return ["localhost", "127.0.0.1", "[::1]", "::1"].includes(new URL(profile.baseUrl).hostname);
  } catch {
    return false;
  }
}

function deriveSelectedRuns(turns: TurnView[]): SelectedRuns {
  const selected: SelectedRuns = {};
  for (const turn of turns) {
    if (turn.runs[0]) selected[turn.id] = turn.runs[0].id;
  }
  for (const child of turns) {
    if (!child.parentRunId) continue;
    const parent = turns.find((turn) => turn.runs.some((run) => run.id === child.parentRunId));
    if (parent) selected[parent.id] = child.parentRunId;
  }
  return selected;
}

function deriveLineage(turns: TurnView[], leafId?: string): TurnView[] {
  if (turns.length === 0) return [];
  const runOwners = new Map<string, TurnView>();
  turns.forEach((turn) => turn.runs.forEach((run) => runOwners.set(run.id, turn)));
  const parentTurnIds = new Set(turns.map((turn) => turn.parentRunId).filter(Boolean).map((runId) => runOwners.get(runId as string)?.id).filter(Boolean));
  let cursor = turns.find((turn) => turn.id === leafId)
    ?? [...turns].reverse().find((turn) => !parentTurnIds.has(turn.id))
    ?? turns.at(-1);
  const lineage: TurnView[] = [];
  const visited = new Set<string>();
  while (cursor && !visited.has(cursor.id)) {
    visited.add(cursor.id);
    lineage.unshift(cursor);
    cursor = cursor.parentRunId ? runOwners.get(cursor.parentRunId) : undefined;
  }
  return lineage;
}

function toInspectorItem(item: ContextPreview["items"][number]): ContextInspectorItem {
  return {
    key: item.id,
    ordinal: item.ordinal,
    label: item.label,
    source: item.source,
    role: item.role,
    content: item.content,
    reason: item.reason,
    estimatedTokens: item.estimatedTokens,
    included: item.included,
    pinned: item.pinned,
  };
}

const providerStreamProtocols = new Set<ProviderStreamProtocol>([
  "openai_sse",
  "ollama_ndjson",
  "anthropic_sse",
  "google_sse",
]);

const providerAuthPlacements = new Set<ProviderAuthPlacement>([
  "none",
  "bearer_header",
  "api_key_header",
  "query_param",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function isStringRecord(value: unknown): value is Record<string, string> {
  return isRecord(value) && Object.values(value).every((entry) => typeof entry === "string");
}

function normalizeProviderSnapshot(
  value: Record<string, unknown>,
): LockedSnapshotProviderMetadata {
  const providerId = typeof value.providerId === "string" ? value.providerId.trim() : "";
  const templateRevision = value.templateRevision;
  const streamProtocol = value.streamProtocol;
  const authPlacement = value.authPlacement;
  const authHeaderName = value.authHeaderName;
  const parameters = isRecord(value.parameters) ? { ...value.parameters } : {};
  const hasHeaderName = typeof authHeaderName === "string" && authHeaderName.trim().length > 0;
  const resolved = Boolean(providerId)
    && typeof templateRevision === "number"
    && Number.isInteger(templateRevision)
    && templateRevision > 0
    && typeof streamProtocol === "string"
    && providerStreamProtocols.has(streamProtocol as ProviderStreamProtocol)
    && typeof authPlacement === "string"
    && providerAuthPlacements.has(authPlacement as ProviderAuthPlacement)
    && isStringRecord(value.additionalHeaders)
    && isRecord(value.parameters)
    && (authPlacement === "none" || hasHeaderName);

  if (!resolved) {
    return {
      providerMetadataStatus: "legacy",
      providerId: "legacy",
      templateRevision: 0,
      streamProtocol: "unknown",
      authPlacement: "unknown",
      authHeaderName: null,
      additionalHeaders: {},
      parameters,
    };
  }

  return {
    providerMetadataStatus: "resolved",
    providerId,
    templateRevision: templateRevision as number,
    streamProtocol: streamProtocol as ProviderStreamProtocol,
    authPlacement: authPlacement as ProviderAuthPlacement,
    authHeaderName: hasHeaderName ? (authHeaderName as string).trim() : null,
    additionalHeaders: { ...(value.additionalHeaders as Record<string, string>) },
    parameters,
  };
}

function normalizeSnapshot(raw: unknown): LockedSnapshot | null {
  if (!isRecord(raw)) return null;
  const value = raw;
  if (typeof value.canonicalHash !== "string" || !value.canonicalHash) return null;
  const providerSnapshot = normalizeProviderSnapshot(value);
  return {
    canonicalHash: value.canonicalHash,
    createdAt: typeof value.createdAt === "number" || typeof value.createdAt === "string"
      ? value.createdAt
      : Date.now(),
    ...providerSnapshot,
    providerName: typeof value.providerName === "string" ? value.providerName : "Provider",
    model: typeof value.model === "string" ? value.model : "model",
    baseUrl: typeof value.baseUrl === "string" ? value.baseUrl : "",
    items: Array.isArray(value.items)
      ? (value.items as ContextPreview["items"]).map(toInspectorItem)
      : [],
  };
}

export function FocusWorkspace({
  bridge,
  initialWorkspaceId,
  initialRunId,
  onOpenRouteMap,
  onOpenDecisions,
  onOpenSettings,
}: FocusWorkspaceProps) {
  const [workspaces, setWorkspaces] = useState<WorkspaceView[]>([]);
  const [detail, setDetail] = useState<WorkspaceDetailView | null>(null);
  const [profiles, setProfiles] = useState<ProviderProfileView[]>([]);
  const [providerId, setProviderId] = useState("");
  const [selectedRuns, setSelectedRuns] = useState<SelectedRuns>({});
  const [activeLeafId, setActiveLeafId] = useState<string>();
  const [activeTurnId, setActiveTurnId] = useState<string>();
  const [draft, setDraft] = useState("");
  const [branchRunId, setBranchRunId] = useState<string | null>(null);
  const [branchDraft, setBranchDraft] = useState("");
  const [preview, setPreview] = useState<ContextPreviewView | null>(null);
  const [pinnedSourceIds, setPinnedSourceIds] = useState<string[]>([]);
  const [excludedSourceIds, setExcludedSourceIds] = useState<string[]>([]);
  const [inspectorOpen, setInspectorOpen] = useState(true);
  const [snapshot, setSnapshot] = useState<LockedSnapshot | null>(null);
  const [snapshotLoading, setSnapshotLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [retryAction, setRetryAction] = useState<RetryAction | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [newWorkspaceOpen, setNewWorkspaceOpen] = useState(false);
  const [newWorkspaceTitle, setNewWorkspaceTitle] = useState("");
  const readingPane = useRef<HTMLDivElement>(null);

  const selectedProfile = profiles.find((profile) => profile.id === providerId) ?? profiles[0];
  const lineage = useMemo(() => deriveLineage(detail?.turns ?? [], activeLeafId), [detail?.turns, activeLeafId]);
  const leafTurn = lineage.at(-1);
  const parentRunId = leafTurn ? selectedRuns[leafTurn.id] ?? leafTurn.runs[0]?.id : undefined;

  const clearError = useCallback(() => {
    setError(null);
    setRetryAction(null);
  }, []);

  const reportError = useCallback((
    reason: unknown,
    fallback: string,
    retry?: RetryAction,
  ) => {
    setError(reason instanceof Error ? reason.message : fallback);
    setRetryAction(reason instanceof DesktopBridgeError && reason.retryable && retry ? retry : null);
  }, []);

  const openWorkspace = useCallback(async (workspaceId: string) => {
    clearError();
    try {
      const next = (await bridge.openWorkspace(workspaceId)) as WorkspaceDetailView;
      const departureTurn = initialRunId
        ? next.turns.find((turn) => turn.runs.some((run) => run.id === initialRunId))
        : undefined;
      const nextSelectedRuns = { ...deriveSelectedRuns(next.turns), ...next.selectedRunIds };
      if (departureTurn && initialRunId) nextSelectedRuns[departureTurn.id] = initialRunId;
      setDetail(next);
      setSelectedRuns(nextSelectedRuns);
      setActiveLeafId(departureTurn?.id);
      setActiveTurnId(departureTurn?.id ?? next.turns.at(-1)?.id);
      setPinnedSourceIds([]);
      setExcludedSourceIds([]);
      setSnapshot(null);
    } catch (reason) {
      reportError(reason, "无法打开本地工作区。");
    }
  }, [bridge, clearError, initialRunId, reportError]);

  useEffect(() => {
    let active = true;
    Promise.all([bridge.listWorkspaces(), bridge.listProviderProfiles()])
      .then(async ([workspaceList, providerList]) => {
        if (!active) return;
        const visible = (workspaceList as WorkspaceView[]).filter((workspace) => !workspace.archived);
        const nextProfiles = providerList as ProviderProfileView[];
        setWorkspaces(visible);
        setProfiles(nextProfiles);
        setProviderId(nextProfiles[0]?.id ?? "");
        const targetId = initialWorkspaceId && visible.some((item) => item.id === initialWorkspaceId)
          ? initialWorkspaceId
          : visible[0]?.id;
        if (targetId) await openWorkspace(targetId);
      })
      .catch((reason: unknown) => {
        if (active) reportError(reason, "无法初始化 ThoughsFlow。");
      })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [bridge, initialWorkspaceId, openWorkspace, reportError]);

  const inspect = useCallback(async (
    prompt: string,
    exactParentRunId: string | null | undefined = parentRunId,
  ) => {
    if (!detail || !selectedProfile) return null;
    const result = (await bridge.inspectContext({
      workspaceId: detail.workspace.id,
      parentRunId: exactParentRunId ?? null,
      prompt,
      providerProfileId: selectedProfile.id,
    })) as ContextPreviewView;
    return result;
  }, [bridge, detail, excludedSourceIds, parentRunId, pinnedSourceIds, selectedProfile]);

  useEffect(() => {
    if (!detail || !selectedProfile) {
      setPreview(null);
      return;
    }
    let active = true;
    const timer = window.setTimeout(() => {
      inspect(draft)
        .then((result) => { if (active && result) setPreview(result); })
        .catch((reason: unknown) => {
          if (!active) return;
          const retryInspection = () => {
            clearError();
            void inspect(draft)
              .then((result) => { if (result) setPreview(result); })
              .catch((nextReason: unknown) => reportError(
                nextReason,
                "无法检查本轮 Context。",
                { label: "重新检查 Context", execute: retryInspection },
              ));
          };
          reportError(
            reason,
            "无法检查本轮 Context。",
            { label: "重新检查 Context", execute: retryInspection },
          );
        });
    }, 120);
    return () => { active = false; window.clearTimeout(timer); };
  }, [clearError, detail, draft, excludedSourceIds, inspect, parentRunId, pinnedSourceIds, reportError, selectedProfile]);

  const updateRun = (runId: string, change: Partial<RunView>) => {
    setDetail((current) => current ? {
      ...current,
      turns: current.turns.map((turn) => ({
        ...turn,
        runs: turn.runs.map((run) => run.id === runId ? { ...run, ...change } : run),
      })),
    } : current);
  };

  const consumeRunEvent = (event: unknown) => {
    if (!event || typeof event !== "object") return;
    const runEvent = event as RunEventView;
    if (!runEvent.runId) return;
    if (runEvent.type === "run-started") {
      updateRun(runEvent.runId, { status: "streaming" });
    } else if (runEvent.type === "text-delta") {
      setDetail((current) => current ? {
        ...current,
        turns: current.turns.map((turn) => ({
          ...turn,
          runs: turn.runs.map((run) => run.id === runEvent.runId
            ? { ...run, status: "streaming", output: `${run.output}${runEvent.text ?? ""}` }
            : run),
        })),
      } : current);
    } else if (runEvent.type === "reasoning-delta") {
      setDetail((current) => current ? {
        ...current,
        turns: current.turns.map((turn) => ({
          ...turn,
          runs: turn.runs.map((run) => run.id === runEvent.runId
            ? { ...run, status: "streaming", reasoning: `${run.reasoning ?? ""}${runEvent.text ?? ""}` }
            : run),
        })),
      } : current);
    } else if (runEvent.type === "usage-updated") {
      updateRun(runEvent.runId, { usage: runEvent.usage });
    } else if (runEvent.type === "run-completed") {
      updateRun(runEvent.runId, { status: "completed", completedAt: new Date().toISOString(), usage: runEvent.usage });
      setNotice(null);
      setBusy(false);
    } else if (runEvent.type === "run-failed") {
      updateRun(runEvent.runId, {
        status: "failed",
        completedAt: runEvent.at ?? new Date().toISOString(),
        error: runEvent.error,
      });
      setNotice(null);
      setBusy(false);
    } else if (runEvent.type === "run-cancelled") {
      updateRun(runEvent.runId, {
        status: "cancelled",
        completedAt: runEvent.at ?? new Date().toISOString(),
      });
      setNotice(null);
      setBusy(false);
    } else if (runEvent.type === "persistence-failed") {
      updateRun(runEvent.runId, {
        status: "interrupted",
        completedAt: runEvent.at ?? new Date().toISOString(),
        error: runEvent.error,
      });
      setNotice(null);
      setBusy(false);
    }
  };

  const bufferedRunEvents = () => {
    const pending: RunEventView[] = [];
    let ready = false;
    return {
      consume: (event: RunEventView) => {
        if (ready) consumeRunEvent(event);
        else pending.push(event);
      },
      release: () => {
        ready = true;
        pending.splice(0).forEach(consumeRunEvent);
      },
    };
  };

  const startTurn = async (prompt: string, exactParentRunId?: string) => {
    if (!detail || !selectedProfile || busy) return;
    clearError();
    setBusy(true);
    try {
      const checked = await inspect(prompt, exactParentRunId);
      if (!checked) throw new Error("无法生成发送前凭证。");
      setPreview(checked);
      if (checked.blocked) throw new Error(checked.warnings[0] || "Context 超过端点限制，发送已阻止。");
      const streamEvents = bufferedRunEvents();
      const started = (await bridge.createTurnAndStartRun({
        workspaceId: detail.workspace.id,
        parentRunId: exactParentRunId ?? null,
        prompt,
        providerProfileId: selectedProfile.id,
        previewHash: checked.hash,
      }, streamEvents.consume)) as { turnId: string; runId: string };
      const optimisticRun: RunView = {
        id: started.runId,
        turnId: started.turnId,
        providerProfileId: selectedProfile.id,
        providerName: selectedProfile.name,
        model: selectedProfile.model,
        baseUrl: selectedProfile.baseUrl,
        status: "connecting",
        output: "",
        createdAt: new Date().toISOString(),
      };
      setDetail((current) => current ? {
        ...current,
        turns: [...current.turns, {
          id: started.turnId,
          workspaceId: current.workspace.id,
          parentRunId: exactParentRunId ?? null,
          prompt,
          createdAt: new Date().toISOString(),
          runs: [optimisticRun],
        }],
      } : current);
      setSelectedRuns((current) => ({ ...current, [started.turnId]: started.runId }));
      setActiveLeafId(started.turnId);
      setActiveTurnId(started.turnId);
      setDraft("");
      setBranchDraft("");
      setBranchRunId(null);
      setNotice("请求凭证已锁定，正在等待 Provider 返回。");
      streamEvents.release();
    } catch (reason) {
      setBusy(false);
      reportError(
        reason,
        "发送失败。",
        { label: "重试发送", execute: () => { void startTurn(prompt, exactParentRunId); } },
      );
    }
  };

  const submitMessage = (event: FormEvent) => {
    event.preventDefault();
    if (draft.trim()) void startTurn(draft.trim(), parentRunId);
  };

  const submitBranch = (event: FormEvent) => {
    event.preventDefault();
    if (branchRunId && branchDraft.trim()) void startTurn(branchDraft.trim(), branchRunId);
  };

  const retryRun = async (run: RunView) => {
    if (!detail || !selectedProfile || busy) return;
    const turn = detail.turns.find((item) => item.id === run.turnId);
    if (!turn) return;
    setBusy(true);
    clearError();
    try {
      const checked = await inspect(turn.prompt, turn.parentRunId ?? null);
      if (!checked || checked.blocked) throw new Error(checked?.warnings[0] || "Context 无法发送。");
      const streamEvents = bufferedRunEvents();
      const started = (await bridge.retryRun({
        runId: run.id,
        providerProfileId: selectedProfile.id,
        previewHash: checked.hash,
      }, streamEvents.consume)) as { runId: string };
      const nextRun: RunView = {
        ...run,
        id: started.runId,
        providerProfileId: selectedProfile.id,
        providerName: selectedProfile.name,
        model: selectedProfile.model,
        baseUrl: selectedProfile.baseUrl,
        status: "connecting",
        output: "",
        createdAt: new Date().toISOString(),
        completedAt: undefined,
        error: undefined,
      };
      setDetail((current) => current ? {
        ...current,
        turns: current.turns.map((item) => item.id === turn.id ? { ...item, runs: [...item.runs, nextRun] } : item),
      } : current);
      setSelectedRuns((current) => ({ ...current, [turn.id]: started.runId }));
      streamEvents.release();
    } catch (reason) {
      setBusy(false);
      reportError(
        reason,
        "重试失败。",
        { label: "重试回答请求", execute: () => { void retryRun(run); } },
      );
    }
  };

  const toggleOverride = async (item: ContextInspectorItem, kind: "included" | "pinned") => {
    if (!detail) return;
    const sourceId = item.key;
    const nextPinned = kind === "pinned" ? (item.pinned ? pinnedSourceIds.filter((id) => id !== sourceId) : [...pinnedSourceIds, sourceId]) : pinnedSourceIds;
    const nextExcluded = kind === "included" ? (item.included ? [...excludedSourceIds, sourceId] : excludedSourceIds.filter((id) => id !== sourceId)) : excludedSourceIds;
    setPinnedSourceIds(nextPinned);
    setExcludedSourceIds(nextExcluded);
    try {
      await bridge.updateContextOverrides({
        workspaceId: detail.workspace.id,
        parentRunId: parentRunId ?? null,
        itemId: sourceId,
        included: kind === "included" ? !item.included : item.included,
        pinned: kind === "pinned" ? !item.pinned : item.pinned,
      });
    } catch (reason) {
      setPinnedSourceIds(pinnedSourceIds);
      setExcludedSourceIds(excludedSourceIds);
      reportError(reason, "上下文调整未保存。");
    }
  };

  const loadSnapshot = async (tab: InspectorTab) => {
    if (tab !== "snapshot") return;
    const selectedRun = [...lineage].reverse().flatMap((turn) => turn.runs).find((run) => selectedRuns[run.turnId] === run.id);
    if (!selectedRun) return;
    setSnapshotLoading(true);
    try {
      setSnapshot(normalizeSnapshot(await bridge.getRunSnapshot(selectedRun.id)));
    } catch (reason) {
      reportError(reason, "无法读取锁定快照。");
    } finally {
      setSnapshotLoading(false);
    }
  };

  const createWorkspace = async (event: FormEvent) => {
    event.preventDefault();
    const name = newWorkspaceTitle.trim();
    if (!name) return;
    try {
      const created = (await bridge.createWorkspace({ name, goal: "" })) as WorkspaceView;
      setWorkspaces((current) => [created, ...current]);
      setNewWorkspaceOpen(false);
      setNewWorkspaceTitle("");
      await openWorkspace(created.id);
    } catch (reason) {
      reportError(reason, "创建工作区失败。");
    }
  };

  const renameWorkspace = async () => {
    if (!detail) return;
    const name = window.prompt("重命名工作区", detail.workspace.name)?.trim();
    if (!name || name === detail.workspace.name) return;
    try {
      const updated = (await bridge.updateWorkspace({ id: detail.workspace.id, name })) as WorkspaceView;
      setDetail({ ...detail, workspace: updated });
      setWorkspaces((current) => current.map((item) => item.id === updated.id ? updated : item));
    } catch (reason) {
      reportError(reason, "重命名失败。");
    }
  };

  const archiveWorkspace = async () => {
    if (!detail) return;
    try {
      await bridge.updateWorkspace({ id: detail.workspace.id, archived: true });
      const remaining = workspaces.filter((item) => item.id !== detail.workspace.id);
      setWorkspaces(remaining);
      setDetail(null);
      if (remaining[0]) await openWorkspace(remaining[0].id);
    } catch (reason) {
      reportError(reason, "归档失败。");
    }
  };

  const onComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if ((event.metaKey || event.ctrlKey) && event.key === "Enter" && draft.trim() && !busy && !preview?.blocked) {
      event.preventDefault();
      void startTurn(draft.trim(), parentRunId);
    }
  };

  if (loading) return <div className="focus-workspace focus-workspace--state"><LoadingState /></div>;

  const contextItems = (preview?.items ?? []).map(toInspectorItem);
  const inspectorRuns: InspectorRun[] = lineage.flatMap((turn) => turn.runs.map((run, index) => ({
    id: run.id,
    label: `${runLabel(index)} · ${run.status}`,
    status: run.status,
    model: run.model,
    createdAt: run.createdAt,
    error: run.error?.message,
  })));
  const providerBoundary = {
    name: selectedProfile?.name ?? "尚未配置 Provider",
    model: selectedProfile?.model ?? "",
    baseUrl: selectedProfile?.baseUrl ?? "未配置",
    local: isLocalProfile(selectedProfile),
  };

  return (
    <div className={`focus-workspace ${inspectorOpen ? "has-inspector" : ""}`}>
      <aside className="focus-sidebar" aria-label="工作区与路线导航">
        <div className="focus-sidebar__brand">
          <Brand />
          <button type="button" aria-label="搜索工作区"><Search size={17} /></button>
        </div>

        <Button className="focus-sidebar__new" tone="quiet" icon={<CirclePlus size={15} />} aria-label="添加工作区" onClick={() => setNewWorkspaceOpen(true)}>
          新建工作区
        </Button>
        {newWorkspaceOpen && (
          <form className="focus-sidebar__new-form" onSubmit={createWorkspace} aria-label="创建工作区">
            <input autoFocus aria-label="工作区名称" value={newWorkspaceTitle} onChange={(event) => setNewWorkspaceTitle(event.target.value)} placeholder="工作区名称" />
            <button type="submit" disabled={!newWorkspaceTitle.trim()} aria-label="确认创建"><ArrowUp size={14} /></button>
          </form>
        )}

        <nav className="focus-sidebar__section" aria-label="工作区">
          <span className="focus-sidebar__heading">工作区</span>
          <div className="focus-sidebar__workspaces">
            {workspaces.map((workspace) => (
              <button
                type="button"
                key={workspace.id}
                className={detail?.workspace.id === workspace.id ? "is-active" : ""}
                onClick={() => void openWorkspace(workspace.id)}
              >
                <FileText size={14} />
                <span>{workspace.name}</span>
              </button>
            ))}
            {workspaces.length === 0 && <p>还没有工作区。</p>}
          </div>
        </nav>

        <nav className="focus-sidebar__section focus-sidebar__route" aria-label="当前路线">
          <span className="focus-sidebar__heading">当前路线 <small>{lineage.length} / {detail?.turns.length ?? 0}</small></span>
          <ol>
            {lineage.map((turn, index) => (
              <li key={turn.id}>
                <button
                  type="button"
                  className={activeTurnId === turn.id ? "is-active" : ""}
                  onClick={() => {
                    setActiveTurnId(turn.id);
                    document.getElementById(`focus-turn-${turn.id}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
                  }}
                >
                  <span>{index + 1}</span>
                  <span>{turn.prompt}</span>
                  {activeTurnId === turn.id && <ChevronRight size={13} />}
                </button>
              </li>
            ))}
          </ol>
        </nav>

        {!!detail?.adjacentBranches.length && (
          <nav className="focus-sidebar__section focus-sidebar__branches" aria-label="相邻分支">
            <span className="focus-sidebar__heading">相邻分支</span>
            {detail.adjacentBranches.map((pointer) => (
              <button key={pointer.runId} type="button" onClick={() => onOpenRouteMap?.(detail.workspace.id, pointer.runId)}>
                <GitBranch size={12} /> {pointer.label}
              </button>
            ))}
          </nav>
        )}

        <div className="focus-sidebar__footer">
          <LocalDataBadge />
          <span><Check size={11} /> 本地已保存</span>
        </div>
      </aside>

      <main className="focus-main">
        {detail ? (
          <>
            <header className="focus-header">
              <div className="focus-header__identity">
                <span><Route size={12} /> 当前路线 · 上下文透明</span>
                <button type="button" onClick={renameWorkspace} title="重命名工作区"><h1>{detail.workspace.name}</h1></button>
                <p>{detail.workspace.goal || "从精确回答继续；旁支不会自动进入本轮 Context。"}</p>
              </div>
              <div className="focus-header__actions">
                <Button icon={<Route size={14} />} onClick={() => onOpenRouteMap?.(detail.workspace.id, parentRunId)}>路线图</Button>
                <Button icon={<Pin size={14} />} onClick={() => onOpenDecisions?.(detail.workspace.id)}>决策</Button>
                <Button icon={<PanelRight size={14} />} aria-label="打开上下文检查器" onClick={() => setInspectorOpen(true)}>
                  上下文 <span className="focus-header__count">{contextItems.filter((item) => item.included).length}</span>
                </Button>
                <button type="button" className="focus-icon-button" aria-label="Provider 设置" onClick={onOpenSettings}><Settings2 size={16} /></button>
                <button type="button" className="focus-icon-button" aria-label="归档工作区" onClick={archiveWorkspace}><Archive size={16} /></button>
              </div>
            </header>

            {error && (
              <div className="focus-notice is-error">
                <ErrorState message={error} />
                <div className="focus-notice__actions">
                  {retryAction && (
                    <button type="button" onClick={retryAction.execute}>{retryAction.label}</button>
                  )}
                  <button type="button" onClick={clearError}>关闭</button>
                </div>
              </div>
            )}
            {notice && <div className="focus-notice" role="status"><Check size={14} /> {notice}<button type="button" onClick={() => setNotice(null)}>关闭</button></div>}

            <div className="focus-reading" ref={readingPane}>
              <div className="focus-reading__intro">
                <div><span>专注路线</span><h2>从问题走到可审查的决策</h2></div>
                <p>连续阅读当前祖先链；点击任意回答版本，即可从那个精确 Run 建立新路线。</p>
              </div>

              <div className="focus-thread" aria-label="当前路线的连续对话">
                {lineage.map((turn, turnIndex) => {
                  const selectedRunId = selectedRuns[turn.id] ?? turn.runs[0]?.id;
                  const selectedRun = turn.runs.find((run) => run.id === selectedRunId) ?? turn.runs[0];
                  if (!selectedRun) return null;
                  const profile = profiles.find((item) => item.id === selectedRun.providerProfileId) ?? selectedProfile;
                  const isGenerating = ["pending", "connecting", "streaming"].includes(selectedRun.status);

                  return (
                    <article id={`focus-turn-${turn.id}`} key={turn.id} className="focus-turn" onMouseEnter={() => setActiveTurnId(turn.id)}>
                      <span className="focus-turn__rail" aria-hidden="true">{turnIndex + 1}</span>
                      <section className="focus-turn__prompt">
                        <div><strong>你</strong><time>{formatTime(turn.createdAt)}</time></div>
                        <SafeMarkdown>{turn.prompt}</SafeMarkdown>
                      </section>
                      <section className="focus-turn__answer">
                        <header>
                          <div><FlowMark /><strong>Flow 回答</strong>{profile && <ProviderDestination name={profile.name} baseUrl={profile.baseUrl} local={isLocalProfile(profile)} compact />}</div>
                          <span>{formatDuration(selectedRun)} · {formatUsage(selectedRun.usage)}</span>
                        </header>

                        {turn.runs.length > 1 && (
                          <div className="focus-run-switcher" aria-label={`${turn.prompt.slice(0, 24)}的回答版本`}>
                            {turn.runs.map((run, runIndex) => (
                              <button
                                key={run.id}
                                type="button"
                                className={run.id === selectedRun.id ? "is-active" : ""}
                                aria-pressed={run.id === selectedRun.id}
                                aria-label={`${runLabel(runIndex)} · ${run.model}`}
                                onClick={() => setSelectedRuns((current) => ({ ...current, [turn.id]: run.id }))}
                              >
                                <span className={`focus-run-switcher__dot is-${run.status}`} />
                                <strong>{runLabel(runIndex)}</strong>
                                <span>{run.model}</span>
                                {run.id === selectedRun.id && <Check size={12} />}
                              </button>
                            ))}
                          </div>
                        )}

                        {selectedRun.error && <div className="focus-turn__run-error" role="alert">{selectedRun.error.message}</div>}
                        <SafeMarkdown>{selectedRun.output || (isGenerating ? "正在等待模型响应…" : "该运行没有输出。")}</SafeMarkdown>

                        <div className="focus-turn__actions">
                          <button type="button" onClick={() => void navigator.clipboard?.writeText(selectedRun.output)}><Copy size={13} /> 复制</button>
                          <button
                            type="button"
                            className={branchRunId === selectedRun.id ? "is-active" : ""}
                            aria-label="从此回答创建分支"
                            disabled={isGenerating}
                            onClick={() => { setBranchRunId((current) => current === selectedRun.id ? null : selectedRun.id); setBranchDraft(""); }}
                          ><GitBranch size={13} /> 从此回答创建分支</button>
                          <button type="button" aria-label="重试回答" disabled={isGenerating} onClick={() => void retryRun(selectedRun)}><RefreshCw size={13} /> 新增回答版本</button>
                          {isGenerating && <button type="button" aria-label="停止生成" onClick={() => void bridge.cancelRun(selectedRun.id)}><Square size={12} /> 停止</button>}
                        </div>

                        {branchRunId === selectedRun.id && (
                          <form className="focus-inline-branch" aria-label="创建精确回答分支" onSubmit={submitBranch}>
                            <div><GitBranch size={13} /> 新分支精确绑定「{runLabel(turn.runs.indexOf(selectedRun))} · {selectedRun.model}」</div>
                            <textarea autoFocus rows={2} value={branchDraft} onChange={(event) => setBranchDraft(event.target.value)} placeholder="沿另一个方向追问…" />
                            <footer><span>该旁支不会污染当前路线</span><Button type="submit" tone="primary" disabled={!branchDraft.trim() || busy} icon={<ArrowUp size={13} />}>创建分支</Button></footer>
                          </form>
                        )}
                      </section>
                    </article>
                  );
                })}
                {lineage.length === 0 && <div className="focus-thread__empty">写下第一个问题，建立这条路线的根 Turn。</div>}
              </div>
            </div>

            <form className="focus-composer" onSubmit={submitMessage}>
              <div className="focus-composer__scope">
                <span><GitBranch size={12} /> {parentRunId ? `从精确 Run ${parentRunId.slice(0, 10)} 继续` : "创建根 Turn"}</span>
                <button type="button" onClick={() => setInspectorOpen(true)}>
                  <FileText size={12} /> {contextItems.filter((item) => item.included).length} 项 Context · 约 {(preview?.estimatedTokens ?? 0).toLocaleString()} tokens
                </button>
              </div>
              <textarea
                rows={3}
                aria-label="消息"
                value={draft}
                onChange={(event) => setDraft(event.target.value)}
                onKeyDown={onComposerKeyDown}
                placeholder="沿当前路线继续，或从上方任意回答创建旁支…"
              />
              <div className="focus-composer__toolbar">
                <div>
                  <select aria-label="Provider" value={providerId} onChange={(event) => setProviderId(event.target.value)} disabled={busy}>
                    {profiles.map((profile) => <option key={profile.id} value={profile.id}>{profile.name} · {profile.model}</option>)}
                  </select>
                  <ProviderDestination {...providerBoundary} compact />
                </div>
                <Button type="submit" tone="primary" disabled={!draft.trim() || busy || !!preview?.blocked || !selectedProfile} icon={busy ? <LoaderCircle className="tf-spin" size={15} /> : <Send size={15} />}>
                  {busy ? "发送中" : "发送"}
                </Button>
              </div>
              {preview?.blocked && <p className="focus-composer__blocked" role="alert">{preview.warnings[0] ?? "Context 超出模型限制，发送已阻止。"}</p>}
            </form>
          </>
        ) : (
          <div className="focus-main__empty">
            {error ? <ErrorState message={error} /> : <><Brand /><h1>建立第一个本地工作区</h1><p>工作区内容保存在本机；模型调用的外发边界会在每次发送前显示。</p><Button tone="primary" icon={<CirclePlus size={15} />} onClick={() => setNewWorkspaceOpen(true)}>新建工作区</Button></>}
          </div>
        )}
      </main>

      <ContextInspector
        open={inspectorOpen && !!detail}
        items={contextItems}
        estimatedTokens={preview?.estimatedTokens ?? 0}
        limitTokens={preview?.limitTokens ?? 0}
        warnings={preview?.warnings ?? []}
        blocked={preview?.blocked ?? false}
        provider={providerBoundary}
        snapshot={snapshot}
        snapshotLoading={snapshotLoading}
        runs={inspectorRuns}
        onClose={() => setInspectorOpen(false)}
        onTabChange={(tab) => void loadSnapshot(tab)}
        onToggleIncluded={(item) => void toggleOverride(item, "included")}
        onTogglePinned={(item) => void toggleOverride(item, "pinned")}
      />
    </div>
  );
}
