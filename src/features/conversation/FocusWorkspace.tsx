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
  Sparkles,
  Square,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent, KeyboardEvent } from "react";
import { DesktopBridgeError, type DesktopBridge } from "../../platform/desktop-bridge";
import type {
  ContextPreview,
  ContextTreeProjection,
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
  ContextMaintenancePanel,
  ContextTree,
  resolveContextBranchId,
  type ManualCheckpointProposal,
  type ProviderCheckpointProposal,
} from "../context-tree";
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
import { CredentialRecovery } from "./CredentialRecovery";
import { useWorkspaceSession } from "./workspace-session";
import "../../shared/tokens/index.css";
import "../../shared/ui/styles.css";
import "./focus-workspace.css";

const clientOperationIdPattern =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

export function nextCheckpointOperationId(
  webviewE2eOperationId: string | undefined,
  randomUuid: () => string = () => crypto.randomUUID(),
) {
  return webviewE2eOperationId
    && clientOperationIdPattern.test(webviewE2eOperationId)
    ? webviewE2eOperationId
    : randomUuid();
}

type WorkspaceView = WorkspaceSummary;
type RunView = ModelRun;
type TurnView = Turn;
type WorkspaceDetailView = WorkspaceDetail;
type ContextPreviewView = ContextPreview;
type ProviderProfileView = ProviderProfile;
type RunEventView = RunEvent;
const inspectorOverlayQuery = "(max-width: 1180px)";

type FocusWorkspaceProps = {
  bridge: DesktopBridge;
  initialWorkspaceId?: string;
  initialRunId?: string;
  onWorkspaceChange?: (workspaceId: string) => void;
  onOpenRouteMap?: (workspaceId: string, runId?: string) => void;
  onOpenDecisions?: (workspaceId: string) => void;
  onOpenSettings?: (providerProfileId?: string) => void;
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

function isLocalBaseUrl(baseUrl: string) {
  try {
    return ["localhost", "127.0.0.1", "[::1]", "::1"].includes(new URL(baseUrl).hostname);
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

function deriveLineage(turns: TurnView[], activeRunId?: string | null): TurnView[] {
  if (turns.length === 0 || activeRunId === null) return [];
  const runOwners = new Map<string, TurnView>();
  turns.forEach((turn) => turn.runs.forEach((run) => runOwners.set(run.id, turn)));
  const parentTurnIds = new Set(
    turns
      .map((turn) => turn.parentRunId)
      .filter(Boolean)
      .map((runId) => runOwners.get(runId as string)?.id)
      .filter(Boolean),
  );
  let cursor = activeRunId
    ? runOwners.get(activeRunId)
    : [...turns].reverse().find((turn) => !parentTurnIds.has(turn.id)) ?? turns.at(-1);
  const lineage: TurnView[] = [];
  const visited = new Set<string>();
  while (cursor && !visited.has(cursor.id)) {
    visited.add(cursor.id);
    lineage.unshift(cursor);
    cursor = cursor.parentRunId ? runOwners.get(cursor.parentRunId) : undefined;
  }
  return lineage;
}

function runPathTo(
  nodes: ContextTreeProjection["nodes"],
  leafRunId: string | null,
) {
  const parentByRun = new Map(nodes.map((node) => [node.runId, node.parentRunId]));
  const path = new Set<string>();
  let cursor = leafRunId;
  while (cursor && !path.has(cursor)) {
    path.add(cursor);
    cursor = parentByRun.get(cursor) ?? null;
  }
  return path;
}

function toInspectorItem(item: ContextPreview["items"][number]): ContextInspectorItem {
  return item;
}

function sourceIdentity(item: ContextInspectorItem) {
  return [
    item.sourceRef.kind,
    item.sourceRef.id ?? "",
    item.contentBlockId ?? "",
  ].join(":");
}

function isContextVersionConflict(reason: unknown) {
  return reason instanceof DesktopBridgeError
    && [
      "context_cursor_conflict",
      "context_draft_conflict",
      "branch_version_conflict",
      "stale_context_version",
    ].includes(reason.code);
}

function isTerminalContextMaintenanceError(reason: unknown) {
  if (!(reason instanceof DesktopBridgeError) || !isRecord(reason.details)) return false;
  return ["failed", "cancelled", "conflicted"].includes(
    String(reason.details.maintenanceStatus ?? ""),
  );
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
  onWorkspaceChange,
  onOpenRouteMap,
  onOpenDecisions,
  onOpenSettings,
}: FocusWorkspaceProps) {
  const session = useWorkspaceSession();
  const initialLocation = useRef({ workspaceId: initialWorkspaceId, runId: initialRunId });
  const [workspaces, setWorkspaces] = useState<WorkspaceView[]>([]);
  const [detail, setDetail] = useState<WorkspaceDetailView | null>(null);
  const [profiles, setProfiles] = useState<ProviderProfileView[]>([]);
  const [providerId, setProviderId] = useState("");
  const [selectedRuns, setSelectedRuns] = useState<SelectedRuns>({});
  const [contextTree, setContextTree] = useState<ContextTreeProjection | null>(null);
  const [contextTreeOpen, setContextTreeOpen] = useState(false);
  const [contextTreeBusy, setContextTreeBusy] = useState(false);
  const [maintenanceOpen, setMaintenanceOpen] = useState(false);
  const [maintenanceBusy, setMaintenanceBusy] = useState(false);
  const [activeTurnId, setActiveTurnId] = useState<string>();
  const [draft, setDraft] = useState("");
  const [branchRunId, setBranchRunId] = useState<string | null>(null);
  const [branchDraft, setBranchDraft] = useState("");
  const [preview, setPreview] = useState<ContextPreviewView | null>(null);
  const [draftVersion, setDraftVersion] = useState<number | null>(null);
  const [inspectorOpen, setInspectorOpen] = useState(() => !window.matchMedia(inspectorOverlayQuery).matches);
  const [snapshot, setSnapshot] = useState<LockedSnapshot | null>(null);
  const [snapshotLoading, setSnapshotLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [retryAction, setRetryAction] = useState<RetryAction | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [newWorkspaceOpen, setNewWorkspaceOpen] = useState(false);
  const [newWorkspaceTitle, setNewWorkspaceTitle] = useState("");
  const [newWorkspaceGoal, setNewWorkspaceGoal] = useState("");
  const readingPane = useRef<HTMLDivElement>(null);
  const manualCheckpointOperation = useRef<{ fingerprint: string; id: string } | null>(null);
  const providerCheckpointOperation = useRef<{ fingerprint: string; id: string } | null>(null);
  const snapshotRequestToken = useRef(0);
  const directStreamRuns = useRef(new Set<string>());
  const activeSend = useRef<{ runId: string | null } | null>(null);
  const receiveRunEvent = useRef<(event: RunEventView) => void>(() => undefined);
  const composerDrafts = useRef(new Map<string, { text: string; branch: string; branchRunId: string | null }>());
  if (detail) composerDrafts.current.set(detail.workspace.id, { text: draft, branch: branchDraft, branchRunId });
  const ownsWorkspace = useMemo(
    () => session.capture(detail?.workspace.id),
    [detail?.workspace.id, session, session.generation],
  );

  useEffect(() => {
    const media = window.matchMedia(inspectorOverlayQuery);
    const closeOnNarrow = (event: MediaQueryListEvent) => {
      if (event.matches) setInspectorOpen(false);
    };
    if (media.matches) setInspectorOpen(false);
    media.addEventListener("change", closeOnNarrow);
    return () => media.removeEventListener("change", closeOnNarrow);
  }, []);

  useEffect(() => {
    if (detail) onWorkspaceChange?.(detail.workspace.id);
  }, [detail?.workspace.id, onWorkspaceChange]);

  const selectedProfile = profiles.find((profile) => profile.id === providerId) ?? profiles[0];
  const parentRunId = contextTree?.cursor.activeRunId ?? null;
  const contextCursorIdentity = contextTree
    ? [
        contextTree.workspaceId,
        contextTree.cursor.activeRunId ?? "root",
        contextTree.cursor.branchId ?? "no-branch",
        contextTree.cursor.version,
      ].join(":")
    : null;
  const contextCursorIdentityRef = useRef(contextCursorIdentity);
  contextCursorIdentityRef.current = contextCursorIdentity;
  const lineage = useMemo(
    () => deriveLineage(detail?.turns ?? [], contextTree?.cursor.activeRunId),
    [contextTree?.cursor.activeRunId, detail?.turns],
  );
  const activeContextNode = contextTree?.nodes.find(
    (node) => node.runId === contextTree.cursor.activeRunId,
  );
  const contextCanContinue = parentRunId === null || Boolean(activeContextNode?.canContinue);
  const activeContextBranch = contextTree?.branches.find(
    (branch) => branch.id === contextTree.cursor.branchId,
  );
  const maintenanceAvailable = Boolean(
    contextTree?.cursor.activeRunId
    && activeContextBranch
    && contextTree.nodes.filter((node) => node.isOnActivePath).length > 1
    && selectedProfile
    && draftVersion !== null,
  );

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

  const applyContextTree = useCallback((
    nextTree: ContextTreeProjection,
    turns: TurnView[],
    baseSelectedRuns: SelectedRuns = {},
  ) => {
    const nextSelectedRuns = {
      ...deriveSelectedRuns(turns),
      ...baseSelectedRuns,
    };
    nextTree.nodes
      .filter((node) => node.isOnActivePath)
      .forEach((node) => {
        nextSelectedRuns[node.turnId] = node.runId;
      });
    setContextTree(nextTree);
    setSelectedRuns(nextSelectedRuns);
    const activeTurn = nextTree.cursor.activeRunId
      ? turns.find((turn) => turn.runs.some((run) => run.id === nextTree.cursor.activeRunId))
      : undefined;
    setActiveTurnId(activeTurn?.id);
  }, []);

  const openWorkspace = useCallback(async (workspaceId: string, targetRunId?: string) => {
    const isCurrentNavigation = session.navigate();
    const isCurrentRead = session.read(workspaceId, "workspace");
    const refreshingCurrentWorkspace = isCurrentRead();
    const canOpen = () => isCurrentNavigation() && (!refreshingCurrentWorkspace || isCurrentRead());
    clearError();
    try {
      const [next, loadedTree] = await Promise.all([
        bridge.openWorkspace(workspaceId) as Promise<WorkspaceDetailView>,
        bridge.getContextTree({ workspaceId }),
      ]);
      if (!canOpen()) return;
      let nextTree = loadedTree;
      if (
        targetRunId
        && nextTree.cursor.activeRunId !== targetRunId
        && nextTree.nodes.some((node) => node.runId === targetRunId)
      ) {
        const departureNode = nextTree.nodes.find((node) => node.runId === targetRunId);
        await bridge.setActiveContext({
          workspaceId,
          runId: targetRunId,
          branchId: departureNode
            ? resolveContextBranchId(nextTree, departureNode.branchIds)
            : null,
          expectedCursorVersion: nextTree.cursor.version,
          expectedDraftVersion: nextTree.draftVersion,
        });
        if (!canOpen()) return;
        nextTree = await bridge.getContextTree({ workspaceId });
      }
      if (!canOpen()) return;
      if (session.activate(workspaceId)) {
        directStreamRuns.current.clear();
        activeSend.current = null;
        const savedDraft = composerDrafts.current.get(workspaceId);
        setDraft(savedDraft?.text ?? "");
        setBusy(false);
        setContextTreeBusy(false);
        setMaintenanceBusy(false);
        setBranchRunId(savedDraft?.branchRunId ?? null);
        setBranchDraft(savedDraft?.branch ?? "");
        setNotice(null);
        manualCheckpointOperation.current = null;
        providerCheckpointOperation.current = null;
      }
      clearError();
      setDetail(next);
      applyContextTree(nextTree, next.turns, next.selectedRunIds);
      setDraftVersion(null);
      setPreview(null);
      setSnapshot(null);
      setMaintenanceOpen(false);
    } catch (reason) {
      if (!canOpen()) return;
      reportError(reason, "无法打开本地工作区。");
    }
  }, [applyContextTree, bridge, clearError, reportError, session]);

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
        const initial = initialLocation.current;
        const targetId = initial.workspaceId && visible.some((item) => item.id === initial.workspaceId)
          ? initial.workspaceId
          : visible[0]?.id;
        if (targetId) await openWorkspace(targetId, initial.runId);
      })
      .catch((reason: unknown) => {
        if (active) reportError(reason, "无法初始化 ThoughtsFlow。");
      })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [bridge, openWorkspace, reportError]);

  useEffect(() => {
    snapshotRequestToken.current += 1;
    setSnapshot(null);
    setSnapshotLoading(false);
  }, [contextCursorIdentity]);

  const inspect = useCallback(async (
    prompt: string,
    exactParentRunId: string | null | undefined = parentRunId,
    exactProviderProfileId: string | undefined = selectedProfile?.id,
    exactDraftVersion: number | null = draftVersion,
    requestedBranchId?: string | null,
    purpose: "preview" | "send-preview" = "preview",
  ) => {
    if (!ownsWorkspace() || !detail || !exactProviderProfileId) return null;
    const isCurrentRead = session.read(detail.workspace.id, purpose);
    const parentNode = exactParentRunId
      ? contextTree?.nodes.find((node) => node.runId === exactParentRunId)
      : undefined;
    const branchId = requestedBranchId !== undefined
      ? requestedBranchId
      : parentNode && contextTree
        ? resolveContextBranchId(contextTree, parentNode.branchIds)
        : null;
    const input = {
      workspaceId: detail.workspace.id,
      parentRunId: exactParentRunId ?? null,
      prompt,
      providerProfileId: exactProviderProfileId,
      branchId,
    };
    try {
      const result = (exactDraftVersion === null
        ? await bridge.inspectContext(input)
        : await bridge.previewContextTransition({
            ...input,
            draftVersion: exactDraftVersion,
          })) as ContextPreviewView;
      return isCurrentRead() ? result : null;
    } catch (reason) {
      if (isCurrentRead()) throw reason;
      return null;
    }
  }, [
    bridge,
    contextTree,
    detail,
    draftVersion,
    parentRunId,
    selectedProfile?.id,
    ownsWorkspace,
    session,
  ]);

  useEffect(() => {
    if (!detail || !selectedProfile) {
      setPreview(null);
      return;
    }
    let active = true;
    const timer = window.setTimeout(() => {
      inspect(draft)
        .then((result) => {
          if (!active || !ownsWorkspace() || !result) return;
          setPreview(result);
          setDraftVersion(result.draftVersion);
        })
        .catch((reason: unknown) => {
          if (!active || !ownsWorkspace()) return;
          const retryInspection = () => {
            if (!ownsWorkspace()) return;
            clearError();
            void inspect(draft)
              .then((result) => {
                if (!ownsWorkspace() || !result) return;
                setPreview(result);
                setDraftVersion(result.draftVersion);
              })
              .catch((nextReason: unknown) => {
                if (ownsWorkspace()) reportError(
                  nextReason,
                  "无法检查本轮 Context。",
                  { label: "重新检查 Context", execute: retryInspection },
                );
              });
          };
          reportError(
            reason,
            "无法检查本轮 Context。",
            { label: "重新检查 Context", execute: retryInspection },
          );
        });
    }, 120);
    return () => { active = false; window.clearTimeout(timer); };
  }, [clearError, detail, draft, inspect, ownsWorkspace, parentRunId, reportError, selectedProfile]);

  const refreshContextTree = useCallback(async () => {
    if (!detail) return null;
    const workspaceId = detail.workspace.id;
    const isCurrent = session.read(workspaceId, "workspace");
    if (!isCurrent()) return null;
    let nextDetail: WorkspaceDetailView;
    let nextTree: ContextTreeProjection;
    try {
      [nextDetail, nextTree] = await Promise.all([
        bridge.openWorkspace(workspaceId) as Promise<WorkspaceDetailView>,
        bridge.getContextTree({ workspaceId }),
      ]);
    } catch (reason) {
      if (isCurrent()) throw reason;
      return null;
    }
    if (!isCurrent()) return null;
    setDetail(nextDetail);
    applyContextTree(nextTree, nextDetail.turns, nextDetail.selectedRunIds);
    return nextTree;
  }, [applyContextTree, bridge, detail, session]);

  const selectActiveContext = async (
    runId: string | null,
    requestedBranchId?: string | null,
    propagateFailure = false,
  ) => {
    if (!ownsWorkspace() || !detail || !contextTree || contextTreeBusy) return;
    const node = runId
      ? contextTree.nodes.find((item) => item.runId === runId)
      : undefined;
    const branchId = requestedBranchId !== undefined
      ? requestedBranchId
      : node
        ? resolveContextBranchId(contextTree, node.branchIds)
        : null;
    setContextTreeBusy(true);
    session.invalidateReads();
    clearError();
    try {
      await bridge.setActiveContext({
        workspaceId: detail.workspace.id,
        runId,
        branchId,
        expectedCursorVersion: contextTree.cursor.version,
        expectedDraftVersion: contextTree.draftVersion,
      });
      if (!ownsWorkspace()) return;
      session.invalidateReads();
      await refreshContextTree();
      if (!ownsWorkspace()) return;
      setDraftVersion(null);
      setPreview(null);
      setSnapshot(null);
    } catch (reason) {
      if (!ownsWorkspace()) return;
      if (isContextVersionConflict(reason)) {
        try {
          await refreshContextTree();
          if (!ownsWorkspace()) return;
          setDraftVersion(null);
        } catch {
          // Preserve the original structured conflict as the actionable error.
        }
      }
      if (!ownsWorkspace()) return;
      reportError(
        reason,
        "无法切换 Context 位置。",
        {
          label: "重试切换 Context",
          execute: () => {
            void selectActiveContext(runId);
          },
        },
      );
      if (propagateFailure) throw reason;
    } finally {
      if (ownsWorkspace()) setContextTreeBusy(false);
    }
  };

  const renameContextBranch = async (
    branchId: string,
    name: string,
    expectedBranchVersion: number,
  ) => {
    if (!ownsWorkspace() || !detail || contextTreeBusy) return;
    setContextTreeBusy(true);
    session.invalidateReads();
    clearError();
    try {
      await bridge.renameBranch({
        workspaceId: detail.workspace.id,
        branchId,
        name,
        expectedBranchVersion,
      });
      if (!ownsWorkspace()) return;
      session.invalidateReads();
      await refreshContextTree();
    } catch (reason) {
      if (!ownsWorkspace()) return;
      if (isContextVersionConflict(reason)) {
        try {
          await refreshContextTree();
        } catch {
          // Preserve the original structured conflict as the actionable error.
        }
      }
      if (!ownsWorkspace()) return;
      reportError(reason, "分支名称未保存。");
    } finally {
      if (ownsWorkspace()) setContextTreeBusy(false);
    }
  };

  const refreshPreviewAfterMaintenance = async () => {
    if (!ownsWorkspace()) return;
    const refreshed = await inspect(
      draft,
      contextTree?.cursor.activeRunId ?? null,
      selectedProfile?.id,
      draftVersion,
    );
    if (ownsWorkspace() && refreshed) {
      setPreview(refreshed);
      setDraftVersion(refreshed.draftVersion);
    }
  };

  const checkpointOperationId = (
    holder: typeof manualCheckpointOperation,
    fingerprint: string,
  ) => {
    if (holder.current?.fingerprint === fingerprint) return holder.current.id;
    const webviewE2eOperationId =
      import.meta.env.VITE_THOUGHSFLOW_WEBVIEW_E2E === "1"
        ? document.documentElement.dataset.thoughtsflowWebviewE2eOperationId
        : undefined;
    const id = nextCheckpointOperationId(webviewE2eOperationId);
    holder.current = { fingerprint, id };
    return id;
  };

  const createManualCheckpoint = async (proposal: ManualCheckpointProposal) => {
    if (
      !ownsWorkspace()
      || !detail
      || !contextTree?.cursor.activeRunId
      || !contextTree.cursor.branchId
      || !activeContextBranch
      || maintenanceBusy
    ) return;
    setMaintenanceBusy(true);
    session.invalidateReads();
    clearError();
    try {
      const operationFingerprint = JSON.stringify({
        workspaceId: detail.workspace.id,
        branchId: contextTree.cursor.branchId,
        cursorVersion: contextTree.cursor.version,
        branchVersion: activeContextBranch.version,
        proposal,
      });
      await bridge.createContextCheckpoint({
        clientOperationId: checkpointOperationId(
          manualCheckpointOperation,
          operationFingerprint,
        ),
        workspaceId: detail.workspace.id,
        branchId: contextTree.cursor.branchId,
        kind: proposal.kind,
        sourceRunIds: proposal.sourceRunIds,
        firstKeptRunId: proposal.firstKeptRunId,
        summary: proposal.summary,
        expectedCursorVersion: contextTree.cursor.version,
        expectedBranchVersion: activeContextBranch.version,
      });
      if (!ownsWorkspace()) return;
      session.invalidateReads();
      await refreshContextTree();
      if (!ownsWorkspace()) return;
      await refreshPreviewAfterMaintenance();
      if (!ownsWorkspace()) return;
      manualCheckpointOperation.current = null;
      setMaintenanceOpen(false);
      setNotice(
        proposal.kind === "branch-summary"
          ? "分支摘要检查点已保存并激活。"
          : "Context 检查点已保存并激活。",
      );
    } catch (reason) {
      if (!ownsWorkspace()) return;
      if (isContextVersionConflict(reason)) {
        try {
          await refreshContextTree();
          if (!ownsWorkspace()) return;
          setDraftVersion(null);
        } catch {
          // Preserve the maintenance proposal and original structured conflict.
        }
      }
      if (!ownsWorkspace()) return;
      reportError(reason, "Context 检查点未保存。");
    } finally {
      if (ownsWorkspace()) setMaintenanceBusy(false);
    }
  };

  const summarizeAndSetActiveContext = async (
    proposal: ProviderCheckpointProposal,
  ) => {
    if (
      !ownsWorkspace()
      || !detail
      || !contextTree?.cursor.activeRunId
      || !contextTree.cursor.branchId
      || !activeContextBranch
      || draftVersion === null
      || maintenanceBusy
    ) return;
    setMaintenanceBusy(true);
    session.invalidateReads();
    clearError();
    try {
      const operationFingerprint = JSON.stringify({
        workspaceId: detail.workspace.id,
        branchId: contextTree.cursor.branchId,
        cursorVersion: contextTree.cursor.version,
        branchVersion: activeContextBranch.version,
        draftVersion,
        proposal,
      });
      await bridge.summarizeAndSetActiveContext({
        clientOperationId: checkpointOperationId(
          providerCheckpointOperation,
          operationFingerprint,
        ),
        workspaceId: detail.workspace.id,
        targetRunId: contextTree.cursor.activeRunId,
        branchId: contextTree.cursor.branchId,
        sourceRunIds: proposal.sourceRunIds,
        firstKeptRunId: proposal.firstKeptRunId,
        summaryPrompt: proposal.summaryPrompt,
        providerProfileId: proposal.providerProfileId,
        expectedCursorVersion: contextTree.cursor.version,
        expectedBranchVersion: activeContextBranch.version,
        expectedDraftVersion: draftVersion,
      });
      if (!ownsWorkspace()) return;
      session.invalidateReads();
      await refreshContextTree();
      if (!ownsWorkspace()) return;
      await refreshPreviewAfterMaintenance();
      if (!ownsWorkspace()) return;
      providerCheckpointOperation.current = null;
      setMaintenanceOpen(false);
      setNotice("摘要已生成，Context 已原子切换。");
    } catch (reason) {
      if (!ownsWorkspace()) return;
      if (isTerminalContextMaintenanceError(reason)) {
        providerCheckpointOperation.current = null;
      }
      if (isContextVersionConflict(reason)) {
        try {
          await refreshContextTree();
          if (!ownsWorkspace()) return;
          setDraftVersion(null);
        } catch {
          // Preserve the maintenance proposal and original structured conflict.
        }
      }
      if (!ownsWorkspace()) return;
      reportError(reason, "摘要失败；Context 位置与检查点均未改变。");
    } finally {
      if (ownsWorkspace()) setMaintenanceBusy(false);
    }
  };

  const cancelContextMaintenance = async () => {
    if (!ownsWorkspace()) return;
    if (!maintenanceBusy) {
      manualCheckpointOperation.current = null;
      providerCheckpointOperation.current = null;
      setMaintenanceOpen(false);
      return;
    }
    const operationId = providerCheckpointOperation.current?.id;
    if (!operationId) return;
    clearError();
    try {
      await bridge.cancelContextMaintenance(operationId);
      if (!ownsWorkspace()) return;
      providerCheckpointOperation.current = null;
      setNotice("已请求取消摘要；检查点和当前 Context 不会移动。");
    } catch (reason) {
      if (!ownsWorkspace()) return;
      reportError(reason, "摘要取消请求失败。");
    }
  };

  const updateRun = (runId: string, change: Partial<RunView>) => {
    setDetail((current) => current ? {
      ...current,
      turns: current.turns.map((turn) => ({
        ...turn,
        runs: turn.runs.map((run) => run.id === runId ? { ...run, ...change } : run),
      })),
    } : current);
    setContextTree((current) => current ? {
      ...current,
      nodes: current.nodes.map((node) => node.runId === runId
        ? {
            ...node,
            ...(change.status ? {
              status: change.status,
              canContinue: change.status === "completed",
            } : {}),
            ...(change.output !== undefined ? { outputPreview: change.output.slice(0, 180) } : {}),
          }
        : node),
    } : current);
  };

  const finishRunEvent = (runId: string, failureMessage: string) => {
    // An adopted older Run can finish while this workspace is starting another one.
    // Its result belongs here, but it cannot unlock or refresh over the active send.
    if (activeSend.current && activeSend.current.runId !== runId) return;
    activeSend.current = null;
    setNotice(null);
    setBusy(false);
    void refreshContextTree().catch((reason: unknown) => {
      if (ownsWorkspace()) reportError(reason, failureMessage);
    });
  };

  const consumeRunEvent = (event: unknown) => {
    if (!ownsWorkspace() || !event || typeof event !== "object") return;
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
      finishRunEvent(runEvent.runId, "Run 已完成，但无法刷新权威工作区状态。");
    } else if (runEvent.type === "run-failed") {
      updateRun(runEvent.runId, {
        status: "failed",
        completedAt: runEvent.at ?? new Date().toISOString(),
        error: runEvent.error,
      });
      finishRunEvent(runEvent.runId, "Run 已失败，但无法刷新权威工作区状态。");
    } else if (runEvent.type === "run-cancelled") {
      updateRun(runEvent.runId, {
        status: "cancelled",
        completedAt: runEvent.at ?? new Date().toISOString(),
      });
      finishRunEvent(runEvent.runId, "Run 已取消，但无法刷新权威工作区状态。");
    } else if (runEvent.type === "persistence-failed") {
      updateRun(runEvent.runId, {
        status: "interrupted",
        completedAt: runEvent.at ?? new Date().toISOString(),
        error: runEvent.error,
      });
      finishRunEvent(runEvent.runId, "Run 持久化失败，且无法刷新权威工作区状态。");
    }
  };

  const bufferedRunEvents = () => {
    const pending: RunEventView[] = [];
    let ready = false;
    return {
      consume: (event: RunEventView) => {
        if (!ownsWorkspace()) return;
        if (ready) consumeRunEvent(event);
        else pending.push(event);
      },
      release: () => {
        ready = true;
        pending.splice(0).forEach(consumeRunEvent);
      },
    };
  };

  // Runs reopened after navigation use a fresh UI owner. Runs started here keep their
  // ACK buffer, so the bridge's broadcast must not apply their deltas a second time.
  receiveRunEvent.current = (event) => {
    if (!directStreamRuns.current.has(event.runId)
      && detail?.turns.some((turn) => turn.runs.some((run) => run.id === event.runId))) {
      consumeRunEvent(event);
    }
  };
  useEffect(() => bridge.subscribeToRunEvents((event) => receiveRunEvent.current(event)), [bridge]);

  const startTurn = async (prompt: string, exactParentRunId?: string | null) => {
    if (!ownsWorkspace() || !detail || !selectedProfile || !contextTree || busy || activeSend.current) return;
    const sending: { runId: string | null } = { runId: null };
    activeSend.current = sending;
    clearError();
    setBusy(true);
    session.invalidateReads();
    try {
      const sourceNode = exactParentRunId
        ? contextTree.nodes.find((node) => node.runId === exactParentRunId)
        : undefined;
      const sourceBranchId = sourceNode
        ? resolveContextBranchId(contextTree, sourceNode.branchIds)
        : null;
      const sourceBranch = contextTree.branches.find(
        (branch) => branch.id === sourceBranchId,
      );
      const checked = await inspect(
        prompt,
        exactParentRunId,
        selectedProfile.id,
        draftVersion,
        sourceBranchId,
        "send-preview",
      );
      if (!ownsWorkspace()) return;
      if (!checked) throw new Error("无法生成发送前凭证。");
      setPreview(checked);
      setDraftVersion(checked.draftVersion);
      if (checked.blocked) throw new Error(checked.warnings[0] || "Context 超过端点限制，发送已阻止。");
      const continuingCurrentBranch =
        sourceBranch?.headRunId === (exactParentRunId ?? null)
          ? sourceBranch
          : undefined;
      const streamEvents = bufferedRunEvents();
      const started = await bridge.createTurnAndStartRun({
        workspaceId: detail.workspace.id,
        parentRunId: exactParentRunId ?? null,
        prompt,
        providerProfileId: selectedProfile.id,
        previewHash: checked.hash,
        branchId: sourceBranch?.id ?? null,
        expectedCursorVersion: contextTree.cursor.version,
        expectedBranchVersion: sourceBranch?.version ?? null,
        expectedDraftVersion: checked.draftVersion,
      }, streamEvents.consume);
      if (!ownsWorkspace()) return;
      sending.runId = started.runId;
      directStreamRuns.current.add(started.runId);
      session.invalidateReads();
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
      setActiveTurnId(started.turnId);
      setDraftVersion(started.draftVersion);
      setContextTree((current) => {
        if (!current) return current;
        const parentPath = runPathTo(current.nodes, exactParentRunId ?? null);
        return {
          ...current,
          draftVersion: started.draftVersion,
          cursor: {
            ...current.cursor,
            activeRunId: started.runId,
            branchId: started.branchId,
            version: started.cursorVersion,
            updatedAt: new Date().toISOString(),
          },
          nodes: [
            ...current.nodes.map((node) => ({
              ...node,
              isActive: false,
              isOnActivePath: parentPath.has(node.runId),
            })),
            {
              runId: started.runId,
              turnId: started.turnId,
              parentRunId: exactParentRunId ?? null,
              prompt,
              title: prompt.trim().slice(0, 48),
              outputPreview: "",
              model: selectedProfile.model,
              status: "connecting" as const,
              createdAt: new Date().toISOString(),
              canContinue: false,
              isActive: true,
              isOnActivePath: true,
              branchIds: [started.branchId],
              checkpointIds: [],
            },
          ],
          edges: [
            ...current.edges.map((edge) => ({
              ...edge,
              isOnActivePath: parentPath.has(edge.targetRunId),
            })),
            {
              id: `edge-${started.runId}`,
              sourceRunId: exactParentRunId ?? null,
              targetRunId: started.runId,
              isOnActivePath: true,
            },
          ],
          branches: [
            ...current.branches
              .filter((branch) => branch.id !== started.branchId)
              .map((branch) => ({ ...branch, isActive: false })),
            {
              id: started.branchId,
              name: continuingCurrentBranch?.name ?? "新分支",
              headRunId: started.runId,
              version: started.branchVersion,
              isActive: true,
            },
          ],
        };
      });
      setDraft((current) => current === draft ? "" : current);
      setBranchDraft((current) => current === branchDraft ? "" : current);
      setBranchRunId(null);
      setNotice("请求凭证已锁定，正在等待 Provider 返回。");
      streamEvents.release();
    } catch (reason) {
      if (!ownsWorkspace()) return;
      activeSend.current = null;
      setBusy(false);
      if (isContextVersionConflict(reason)) {
        try {
          await refreshContextTree();
          if (!ownsWorkspace()) return;
          setDraftVersion(null);
          setPreview(null);
        } catch {
          // Keep the original structured conflict as the actionable error.
        }
      }
      if (!ownsWorkspace()) return;
      reportError(
        reason,
        "发送失败。",
        { label: "重试发送", execute: () => { void startTurn(prompt, exactParentRunId); } },
      );
    }
  };

  const submitMessage = (event: FormEvent) => {
    event.preventDefault();
    if (draft.trim() && contextCanContinue) void startTurn(draft.trim(), parentRunId);
  };

  const submitBranch = (event: FormEvent) => {
    event.preventDefault();
    if (branchRunId && branchDraft.trim()) void startTurn(branchDraft.trim(), branchRunId);
  };

  const retryRun = async (
    run: RunView,
    retryProviderProfileId: string,
    retryCredentialId?: string,
    propagateFailure = false,
  ) => {
    if (!ownsWorkspace() || !detail || !contextTree || busy || activeSend.current) return;
    const turn = detail.turns.find((item) => item.id === run.turnId);
    if (!turn) return;
    const sending: { runId: string | null } = { runId: null };
    activeSend.current = sending;
    const retryProfile = profiles.find((profile) => profile.id === retryProviderProfileId);
    setBusy(true);
    session.invalidateReads();
    clearError();
    try {
      const retryNode = contextTree.nodes.find((node) => node.runId === run.id);
      const retryBranchId = retryNode
        ? resolveContextBranchId(contextTree, retryNode.branchIds)
        : null;
      const retryBranch = contextTree.branches.find(
        (branch) => branch.id === retryBranchId,
      );
      const checked = await inspect(
        turn.prompt,
        turn.parentRunId ?? null,
        retryProviderProfileId,
        draftVersion,
        retryBranchId,
        "send-preview",
      );
      if (!ownsWorkspace()) return;
      if (!checked || checked.blocked) throw new Error(checked?.warnings[0] || "Context 无法发送。");
      if (checked.providerProfileId !== retryProviderProfileId) {
        throw new Error("Context 预览的 Provider 已变化，请重新确认后重试。");
      }
      const streamEvents = bufferedRunEvents();
      const started = await bridge.retryRun({
        runId: run.id,
        providerProfileId: retryProviderProfileId,
        previewHash: checked.hash,
        ...(retryCredentialId ? { credentialId: retryCredentialId } : {}),
        branchId: retryBranch?.id ?? null,
        expectedCursorVersion: contextTree.cursor.version,
        expectedBranchVersion: retryBranch?.version ?? null,
        expectedDraftVersion: checked.draftVersion,
      }, streamEvents.consume);
      if (!ownsWorkspace()) return;
      sending.runId = started.runId;
      directStreamRuns.current.add(started.runId);
      session.invalidateReads();
      const nextRun: RunView = {
        ...run,
        id: started.runId,
        providerProfileId: retryProviderProfileId,
        providerName: retryProfile?.name ?? run.providerName,
        model: retryProfile?.model ?? run.model,
        baseUrl: retryProfile?.baseUrl ?? run.baseUrl,
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
      setDraftVersion(started.draftVersion);
      setContextTree((current) => {
        if (!current) return current;
        const parentPath = runPathTo(current.nodes, turn.parentRunId);
        return {
          ...current,
          draftVersion: started.draftVersion,
          cursor: {
            ...current.cursor,
            activeRunId: started.runId,
            branchId: started.branchId,
            version: started.cursorVersion,
            updatedAt: new Date().toISOString(),
          },
          nodes: [
            ...current.nodes.map((node) => ({
              ...node,
              isActive: false,
              isOnActivePath: parentPath.has(node.runId),
            })),
            {
              runId: started.runId,
              turnId: turn.id,
              parentRunId: turn.parentRunId,
              prompt: turn.prompt,
              title: turn.title?.trim() || turn.prompt.trim().slice(0, 48),
              outputPreview: "",
              model: retryProfile?.model ?? run.model,
              status: "connecting" as const,
              createdAt: new Date().toISOString(),
              canContinue: false,
              isActive: true,
              isOnActivePath: true,
              branchIds: [started.branchId],
              checkpointIds: [],
            },
          ],
          edges: [
            ...current.edges.map((edge) => ({
              ...edge,
              isOnActivePath: parentPath.has(edge.targetRunId),
            })),
            {
              id: `edge-${started.runId}`,
              sourceRunId: turn.parentRunId,
              targetRunId: started.runId,
              isOnActivePath: true,
            },
          ],
          branches: [
            ...current.branches
              .filter((branch) => branch.id !== started.branchId)
              .map((branch) => ({ ...branch, isActive: false })),
            {
              id: started.branchId,
              name: "重试分支",
              headRunId: started.runId,
              version: started.branchVersion,
              isActive: true,
            },
          ],
        };
      });
      streamEvents.release();
    } catch (reason) {
      if (!ownsWorkspace()) return;
      activeSend.current = null;
      setBusy(false);
      if (isContextVersionConflict(reason)) {
        try {
          await refreshContextTree();
          if (!ownsWorkspace()) return;
          setDraftVersion(null);
          setPreview(null);
        } catch {
          // Keep the original structured conflict as the actionable error.
        }
      }
      if (!ownsWorkspace()) return;
      reportError(
        reason,
        "重试失败。",
        {
          label: "重试回答请求",
          execute: () => {
            void retryRun(run, retryProviderProfileId, retryCredentialId);
          },
        },
      );
      if (propagateFailure) throw reason;
    }
  };

  const toggleOverride = async (item: ContextInspectorItem, kind: "included" | "pinned") => {
    if (!ownsWorkspace() || !detail || !preview || item.mandatory) return;
    const targetIdentity = sourceIdentity(item);
    const sourceItems = new Map<string, ContextInspectorItem>();
    [...preview.rawItems, ...preview.items].forEach((candidate) => {
      sourceItems.set(sourceIdentity(candidate), candidate);
    });
    const nextItems = [...sourceItems.values()].map((candidate) => {
      const isTarget = sourceIdentity(candidate) === targetIdentity;
      return {
        sourceRef: candidate.sourceRef,
        contentBlockId: candidate.contentBlockId,
        included: isTarget && kind === "included"
          ? !candidate.included
          : candidate.included,
        pinned: isTarget && kind === "pinned"
          ? !candidate.pinned
          : candidate.pinned,
      };
    });
    try {
      const updated = await bridge.updateContextDraft({
        workspaceId: detail.workspace.id,
        parentRunId: parentRunId ?? null,
        expectedDraftVersion: preview.draftVersion,
        items: nextItems,
      });
      if (!ownsWorkspace()) return;
      session.invalidateReads();
      setDraftVersion(updated.draftVersion);
      setContextTree((current) => current ? {
        ...current,
        draftVersion: updated.draftVersion,
      } : current);
      const refreshed = await inspect(
        draft,
        parentRunId,
        selectedProfile?.id,
        updated.draftVersion,
      );
      if (ownsWorkspace() && refreshed) {
        setPreview(refreshed);
        setDraftVersion(refreshed.draftVersion);
      }
    } catch (reason) {
      if (!ownsWorkspace()) return;
      if (isContextVersionConflict(reason)) {
        setDraftVersion(null);
        try {
          await refreshContextTree();
        } catch {
          // Keep the original conflict visible and leave the composer draft untouched.
        }
      }
      if (!ownsWorkspace()) return;
      reportError(reason, "上下文调整未保存。");
    }
  };

  const loadSnapshot = async (tab: InspectorTab) => {
    if (!ownsWorkspace() || tab !== "snapshot") return;
    const selectedRun = detail?.turns
      .flatMap((turn) => turn.runs)
      .find((run) => run.id === contextTree?.cursor.activeRunId);
    if (!selectedRun) return;
    const requestToken = snapshotRequestToken.current + 1;
    snapshotRequestToken.current = requestToken;
    const requestContextIdentity = contextCursorIdentityRef.current;
    const isCurrentRead = session.read(detail?.workspace.id, "snapshot");
    setSnapshot(null);
    setSnapshotLoading(true);
    try {
      const loadedSnapshot = await bridge.getRunSnapshot(selectedRun.id);
      if (
        !isCurrentRead()
        || snapshotRequestToken.current !== requestToken
        || contextCursorIdentityRef.current !== requestContextIdentity
        || loadedSnapshot.runId !== selectedRun.id
      ) return;
      setSnapshot(normalizeSnapshot(loadedSnapshot));
    } catch (reason) {
      if (
        !isCurrentRead()
        || snapshotRequestToken.current !== requestToken
        || contextCursorIdentityRef.current !== requestContextIdentity
      ) return;
      reportError(reason, "无法读取锁定快照。");
    } finally {
      if (
        ownsWorkspace()
        && snapshotRequestToken.current === requestToken
        && contextCursorIdentityRef.current === requestContextIdentity
      ) {
        setSnapshotLoading(false);
      }
    }
  };

  const createWorkspace = async (event: FormEvent) => {
    event.preventDefault();
    const name = newWorkspaceTitle.trim();
    const goal = newWorkspaceGoal.trim();
    if (!ownsWorkspace() || !name || !goal) return;
    try {
      const created = (await bridge.createWorkspace({ name, goal })) as WorkspaceView;
      if (!ownsWorkspace()) return;
      setWorkspaces((current) => [created, ...current]);
      setNewWorkspaceOpen(false);
      setNewWorkspaceTitle("");
      setNewWorkspaceGoal("");
      await openWorkspace(created.id);
    } catch (reason) {
      if (!ownsWorkspace()) return;
      reportError(reason, "创建工作区失败。");
    }
  };

  const renameWorkspace = async () => {
    if (!ownsWorkspace() || !detail) return;
    const name = window.prompt("重命名工作区", detail.workspace.name)?.trim();
    if (!name || name === detail.workspace.name) return;
    try {
      const updated = (await bridge.updateWorkspace({ id: detail.workspace.id, name })) as WorkspaceView;
      if (!ownsWorkspace()) return;
      setDetail((current) => current ? { ...current, workspace: updated } : current);
      setWorkspaces((current) => current.map((item) => item.id === updated.id ? updated : item));
    } catch (reason) {
      if (!ownsWorkspace()) return;
      reportError(reason, "重命名失败。");
    }
  };

  const archiveWorkspace = async () => {
    if (!ownsWorkspace() || !detail) return;
    try {
      await bridge.updateWorkspace({ id: detail.workspace.id, archived: true });
      if (!ownsWorkspace()) return;
      const remaining = workspaces.filter((item) => item.id !== detail.workspace.id);
      setWorkspaces(remaining);
      setDetail(null);
      if (remaining[0]) await openWorkspace(remaining[0].id);
      else session.activate(undefined);
    } catch (reason) {
      if (!ownsWorkspace()) return;
      reportError(reason, "归档失败。");
    }
  };

  const onComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (
      (event.metaKey || event.ctrlKey)
      && event.key === "Enter"
      && draft.trim()
      && !busy
      && !preview?.blocked
      && contextCanContinue
    ) {
      event.preventDefault();
      void startTurn(draft.trim(), parentRunId);
    }
  };

  if (loading) return <div className="focus-workspace focus-workspace--state"><LoadingState /></div>;

  const contextItems = (preview?.items ?? []).map(toInspectorItem);
  const rawContextItems = (preview?.rawItems ?? preview?.items ?? []).map(toInspectorItem);
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
    local: isLocalBaseUrl(selectedProfile?.baseUrl ?? ""),
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
            <input aria-label="工作区目标" value={newWorkspaceGoal} onChange={(event) => setNewWorkspaceGoal(event.target.value)} placeholder="需要达成的可验证目标" />
            <button type="submit" disabled={!newWorkspaceTitle.trim() || !newWorkspaceGoal.trim()} aria-label="确认创建"><ArrowUp size={14} /></button>
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

        <button
          aria-expanded={contextTreeOpen}
          aria-label="打开 Context Tree"
          className="focus-sidebar__tree-button"
          disabled={!contextTree}
          onClick={() => setContextTreeOpen(true)}
          type="button"
        >
          <GitBranch aria-hidden="true" size={14} />
          <span>Context Tree</span>
          <small>{contextTree?.nodes.length ?? 0}</small>
        </button>

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
              <button
                key={pointer.runId}
                type="button"
                onClick={() => {
                  void selectActiveContext(pointer.runId, undefined, true)
                    .then(() => { if (ownsWorkspace()) onOpenRouteMap?.(detail.workspace.id); })
                    .catch(() => undefined);
                }}
              >
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

      {contextTreeOpen && contextTree ? (
        <aside className="focus-context-tree-drawer" aria-label="Context Tree 抽屉">
          <button
            aria-label="关闭 Context Tree"
            className="focus-context-tree-drawer__close"
            onClick={() => setContextTreeOpen(false)}
            type="button"
          >
            <X aria-hidden="true" size={15} />
          </button>
          <ContextTree
            busy={contextTreeBusy}
            onRenameBranch={renameContextBranch}
            onSelect={selectActiveContext}
            projection={contextTree}
          />
        </aside>
      ) : null}

      {maintenanceOpen && contextTree ? (
        <div className="focus-context-maintenance">
          <ContextMaintenancePanel
            busy={maintenanceBusy}
            estimatedTokens={preview?.estimatedTokens ?? 0}
            nodes={contextTree.nodes}
            onCancel={cancelContextMaintenance}
            onCreateManual={createManualCheckpoint}
            onSummarize={summarizeAndSetActiveContext}
            profiles={profiles}
          />
        </div>
      ) : null}

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
                <Button icon={<Route size={14} />} onClick={() => onOpenRouteMap?.(detail.workspace.id)}>路线图</Button>
                <Button icon={<Pin size={14} />} onClick={() => onOpenDecisions?.(detail.workspace.id)}>决策</Button>
                <Button
                  aria-label="准备 Context 压缩"
                  disabled={!maintenanceAvailable}
                  icon={<Sparkles size={14} />}
                  onClick={() => setMaintenanceOpen(true)}
                >
                  压缩
                </Button>
                <Button icon={<PanelRight size={14} />} aria-label="打开上下文检查器" onClick={() => setInspectorOpen(true)}>
                  上下文 <span className="focus-header__count">{contextItems.filter((item) => item.included).length}</span>
                </Button>
                <button
                  type="button"
                  className="focus-icon-button"
                  aria-label="Provider 设置"
                  onClick={() => onOpenSettings?.()}
                ><Settings2 size={16} /></button>
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
                          <div>
                            <FlowMark />
                            <strong>Flow 回答</strong>
                            {selectedRun.providerName && selectedRun.baseUrl && (
                              <ProviderDestination
                                name={selectedRun.providerName}
                                baseUrl={selectedRun.baseUrl}
                                local={isLocalBaseUrl(selectedRun.baseUrl)}
                                compact
                              />
                            )}
                          </div>
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
                                disabled={contextTreeBusy}
                                onClick={() => void selectActiveContext(run.id)}
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
                        <CredentialRecovery
                          bridge={bridge}
                          run={selectedRun}
                          disabled={busy || isGenerating}
                          onRetryWithCredential={(exactProviderProfileId, credentialId) =>
                            retryRun(selectedRun, exactProviderProfileId, credentialId, true)}
                          onOpenSettings={onOpenSettings}
                        />

                        <div className="focus-turn__actions">
                          <button type="button" onClick={() => void navigator.clipboard?.writeText(selectedRun.output)}><Copy size={13} /> 复制</button>
                          <button
                            type="button"
                            className={branchRunId === selectedRun.id ? "is-active" : ""}
                            aria-label="从此回答创建分支"
                            disabled={isGenerating || selectedRun.status !== "completed"}
                            onClick={() => { setBranchRunId((current) => current === selectedRun.id ? null : selectedRun.id); setBranchDraft(""); }}
                          ><GitBranch size={13} /> 从此回答创建分支</button>
                          <button
                            type="button"
                            aria-label="重试回答"
                            disabled={isGenerating || !selectedProfile}
                            onClick={() => {
                              if (selectedProfile) void retryRun(selectedRun, selectedProfile.id);
                            }}
                          ><RefreshCw size={13} /> 新增回答版本</button>
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
                disabled={!contextCanContinue}
              />
              <div className="focus-composer__toolbar">
                <div>
                  <select aria-label="Provider" value={providerId} onChange={(event) => setProviderId(event.target.value)} disabled={busy}>
                    {profiles.map((profile) => <option key={profile.id} value={profile.id}>{profile.name} · {profile.model}</option>)}
                  </select>
                  <ProviderDestination {...providerBoundary} compact />
                </div>
                <Button type="submit" tone="primary" disabled={!draft.trim() || busy || !!preview?.blocked || !selectedProfile || !contextCanContinue} icon={busy ? <LoaderCircle className="tf-spin" size={15} /> : <Send size={15} />}>
                  {busy ? "发送中" : "发送"}
                </Button>
              </div>
              {!contextCanContinue ? (
                <p className="focus-composer__blocked" role="status">
                  当前 Run 不可继续；请重试此回答，或在 Context Tree 中选择可继承的祖先。
                </p>
              ) : null}
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
        rawItems={rawContextItems}
        draftVersion={preview?.draftVersion ?? draftVersion ?? 0}
        appliedCheckpoint={preview?.appliedCheckpoint ?? null}
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
