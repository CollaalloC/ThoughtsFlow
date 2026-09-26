import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  AgentEnvironment,
  AgentMission,
  AgentOperation,
  AgentOutput,
  AgentSnapshot,
  CreateAgentMissionInput,
  ReadAgentOutputInput,
  ReconnectAgentMissionInput,
  ReleaseAgentWorkerInput,
  ReplyToAgentInput,
  StartAgentTaskInput,
  ApiEnvelope,
  CompareRunsResult,
  ContextBranchView,
  ContextCheckpointProvenance,
  ContextCheckpointProviderSnapshot,
  ContextCheckpointView,
  ContextCursor,
  ContextPreview,
  ContextTreeProjection,
  CreateContextCheckpointInput,
  CreateTurnAndStartRunInput,
  DecisionMark,
  DecisionStatus,
  ExportResult,
  InspectContextInput,
  ListProviderModelsInput,
  ProviderModelInfo,
  ProviderProfile,
  ProviderTemplate,
  PreviewContextTransitionInput,
  RenameBranchInput,
  RetryRunInput,
  RouteProjection,
  RunEvent,
  RunHandle,
  RunSnapshot,
  SaveProviderProfileInput,
  SetActiveContextInput,
  SessionCredentialSummary,
  SummarizeAndSetActiveContextInput,
  SummarizeAndSetActiveContextResult,
  UpdateContextDraftInput,
  UpdateContextDraftResult,
  WorkspaceDetail,
  WorkspaceSummary,
} from "../shared/contracts";

export interface DesktopBridge {
  agentEnvironment(): Promise<AgentEnvironment>;
  openAgentRuntime(): Promise<AgentEnvironment>;
  listAgentMissions(workspaceId: string): Promise<AgentMission[]>;
  createAgentMission(input: CreateAgentMissionInput): Promise<AgentMission>;
  getAgentSnapshot(missionId: string): Promise<AgentSnapshot>;
  startAgentTask(input: StartAgentTaskInput): Promise<AgentOperation>;
  replyToAgent(input: ReplyToAgentInput): Promise<AgentOperation>;
  releaseAgentWorker(input: ReleaseAgentWorkerInput): Promise<AgentOperation>;
  reconnectAgentMission(input: ReconnectAgentMissionInput): Promise<AgentOperation>;
  listAgentOperations(missionId: string): Promise<AgentOperation[]>;
  readAgentOutput(input: ReadAgentOutputInput): Promise<AgentOutput>;
  listWorkspaces(): Promise<WorkspaceSummary[]>;
  createWorkspace(input: {
    name: string;
    goal: string;
    systemPrompt?: string;
  }): Promise<WorkspaceSummary>;
  openWorkspace(id: string): Promise<WorkspaceDetail>;
  updateWorkspace(input: {
    id: string;
    name?: string;
    goal?: string;
    systemPrompt?: string;
    archived?: boolean;
  }): Promise<WorkspaceSummary>;
  inspectContext(input: InspectContextInput): Promise<ContextPreview>;
  previewContextTransition(input: PreviewContextTransitionInput): Promise<ContextPreview>;
  getContextTree(input: { workspaceId: string }): Promise<ContextTreeProjection>;
  setActiveContext(input: SetActiveContextInput): Promise<ContextCursor>;
  renameBranch(input: RenameBranchInput): Promise<ContextBranchView>;
  updateContextDraft(input: UpdateContextDraftInput): Promise<UpdateContextDraftResult>;
  createContextCheckpoint(input: CreateContextCheckpointInput): Promise<ContextCheckpointView>;
  summarizeAndSetActiveContext(
    input: SummarizeAndSetActiveContextInput,
  ): Promise<SummarizeAndSetActiveContextResult>;
  cancelContextMaintenance(clientOperationId: string): Promise<void>;
  createTurnAndStartRun(
    input: CreateTurnAndStartRunInput,
    onEvent: (event: RunEvent) => void,
  ): Promise<RunHandle>;
  retryRun(input: RetryRunInput, onEvent: (event: RunEvent) => void): Promise<RunHandle>;
  cancelRun(runId: string): Promise<void>;
  getRunSnapshot(runId: string): Promise<RunSnapshot>;
  updateContextOverrides(input: {
    workspaceId: string;
    parentRunId: string | null;
    itemId: string;
    included: boolean;
    pinned: boolean;
  }): Promise<void>;
  getRouteProjection(input: {
    workspaceId: string;
    currentRunId?: string;
  }): Promise<RouteProjection>;
  updateViewState(input: {
    workspaceId: string;
    turnId: string;
    x: number;
    y: number;
    collapsed: boolean;
  }): Promise<void>;
  compareRuns(input: { leftRunId: string; rightRunId: string }): Promise<CompareRunsResult>;
  markDecision(input: {
    workspaceId: string;
    runId: string;
    status: DecisionStatus;
    reason: string;
  }): Promise<DecisionMark>;
  exportDecisionPacket(input: { workspaceId: string }): Promise<ExportResult>;
  listProviderTemplates(): Promise<ProviderTemplate[]>;
  listProviderProfiles(): Promise<ProviderProfile[]>;
  listProviderModels(input: ListProviderModelsInput): Promise<ProviderModelInfo[]>;
  listSessionCredentials(input: {
    providerProfileId: string;
  }): Promise<SessionCredentialSummary[]>;
  saveProviderProfile(
    input: SaveProviderProfileInput,
    initialCredential?: { label: string; credential: string },
  ): Promise<ProviderProfile>;
  setSessionCredential(input: {
    providerProfileId: string;
    credentialId?: string;
    credentialLabel: string;
    credential: string;
  }): Promise<SessionCredentialSummary[]>;
  activateSessionCredential(input: {
    providerProfileId: string;
    credentialId: string;
  }): Promise<SessionCredentialSummary[]>;
  reorderSessionCredentials(input: {
    providerProfileId: string;
    orderedCredentialIds: string[];
  }): Promise<SessionCredentialSummary[]>;
  removeSessionCredential(input: {
    providerProfileId: string;
    credentialId: string;
  }): Promise<SessionCredentialSummary[]>;
  testProviderConnection(input: {
    providerProfileId: string;
  }): Promise<{ ok: boolean; message: string }>;
  subscribeToRunEvents(listener: (event: RunEvent) => void): () => void;
}

type Unlisten = () => void;

export class DesktopBridgeError extends Error {
  readonly code: string;
  readonly retryable: boolean;
  readonly details: unknown;

  constructor(input: {
    code: string;
    message: string;
    retryable?: boolean;
    details?: unknown;
  }) {
    super(input.message);
    this.name = "DesktopBridgeError";
    this.code = input.code;
    this.retryable = input.retryable ?? false;
    this.details = input.details ?? null;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function toSafeSessionCredentialSummaries(
  summaries: SessionCredentialSummary[],
): SessionCredentialSummary[] {
  return summaries.map(({ credentialId, label, order, isActive }) => ({
    credentialId,
    label,
    order,
    isActive,
  }));
}

export function normalizeDesktopBridgeError(reason: unknown): DesktopBridgeError {
  if (reason instanceof DesktopBridgeError) return reason;
  if (isRecord(reason) && typeof reason.code === "string" && typeof reason.message === "string") {
    return new DesktopBridgeError({
      code: reason.code,
      message: reason.message,
      retryable: typeof reason.retryable === "boolean" ? reason.retryable : false,
      details: reason.details,
    });
  }
  if (reason instanceof Error) {
    return new DesktopBridgeError({
      code: "desktop_bridge_error",
      message: reason.message,
      details: { name: reason.name },
    });
  }
  return new DesktopBridgeError({
    code: "desktop_bridge_error",
    message: typeof reason === "string" ? reason : "Desktop command failed",
    details: reason,
  });
}

type InvokeCommand = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export function createDesktopBridge(invokeCommand: InvokeCommand = invoke): DesktopBridge {
  const listeners = new Set<(event: RunEvent) => void>();
  const dispatch = (event: RunEvent) => listeners.forEach((listener) => listener(event));
  const request = async <T>(command: string, args?: Record<string, unknown>) => {
    try {
      const response = await invokeCommand<ApiEnvelope<T>>(command, args);
      if (response.apiVersion !== 1) {
        throw new DesktopBridgeError({
          code: "unsupported_api_version",
          message: `Unsupported DesktopBridge API version: ${response.apiVersion}`,
        });
      }
      return response.data;
    } catch (reason) {
      throw normalizeDesktopBridgeError(reason);
    }
  };

  const streamingInvoke = async <TInput extends object>(
    command: string,
    input: TInput,
    onEvent: (event: RunEvent) => void,
  ) => {
    const channel = new Channel<RunEvent>();
    channel.onmessage = (event) => {
      onEvent(event);
      dispatch(event);
    };
    return request<RunHandle>(command, { input, onEvent: channel });
  };

  return {
    agentEnvironment: () => request("agent_environment"),
    openAgentRuntime: () => request("agent_open_runtime"),
    listAgentMissions: (workspaceId) => request("agent_list_missions", { workspaceId }),
    createAgentMission: (input) => request("agent_create_mission", { input }),
    getAgentSnapshot: (missionId) => request("agent_snapshot", { missionId }),
    startAgentTask: (input) => request("agent_start_task", { input }),
    replyToAgent: (input) => request("agent_reply", { input }),
    releaseAgentWorker: (input) => request("agent_release_worker", { input }),
    reconnectAgentMission: (input) => request("agent_reconnect", { input }),
    listAgentOperations: (missionId) => request("agent_operations", { missionId }),
    readAgentOutput: (input) => request("agent_read_output", { input }),
    listWorkspaces: () => request("list_workspaces"),
    createWorkspace: (input) => request("create_workspace", { input }),
    openWorkspace: (id) => request("open_workspace", { id }),
    updateWorkspace: (input) => request("update_workspace", { input }),
    inspectContext: (input) => request("inspect_context", { input }),
    previewContextTransition: (input) => request("preview_context_transition", { input }),
    getContextTree: (input) => request("get_context_tree", { input }),
    setActiveContext: (input) => request("set_active_context", { input }),
    renameBranch: (input) => request("rename_branch", { input }),
    updateContextDraft: (input) => request("update_context_draft", { input }),
    createContextCheckpoint: (input) => request("create_context_checkpoint", { input }),
    summarizeAndSetActiveContext: (input) =>
      request("summarize_and_set_active_context", { input }),
    cancelContextMaintenance: (clientOperationId) =>
      request("cancel_context_maintenance", { clientOperationId }),
    createTurnAndStartRun: (input, onEvent) =>
      streamingInvoke("create_turn_and_start_run", input, onEvent),
    retryRun: (input, onEvent) => streamingInvoke("retry_run", input, onEvent),
    cancelRun: (runId) => request("cancel_run", { runId }),
    getRunSnapshot: (runId) => request("get_run_snapshot", { runId }),
    updateContextOverrides: (input) => request("update_context_overrides", { input }),
    getRouteProjection: (input) => request("get_route_projection", { input }),
    updateViewState: (input) => request("update_view_state", { input }),
    compareRuns: (input) => request("compare_runs", { input }),
    markDecision: (input) => request("mark_decision", { input }),
    exportDecisionPacket: (input) => request("export_decision_packet", { input }),
    listProviderTemplates: () => request("list_provider_templates"),
    listProviderProfiles: () => request("list_provider_profiles"),
    listProviderModels: (input) => request("list_provider_models", { input }),
    listSessionCredentials: async (input) =>
      toSafeSessionCredentialSummaries(
        await request<SessionCredentialSummary[]>("list_session_credentials", { input }),
      ),
    saveProviderProfile: (input, initialCredential) =>
      request("save_provider_profile", {
        input: {
          ...input,
          ...(initialCredential === undefined
            ? {}
            : {
                sessionCredentialLabel: initialCredential.label,
                sessionCredential: initialCredential.credential,
              }),
        },
      }),
    setSessionCredential: async (input) =>
      toSafeSessionCredentialSummaries(
        await request<SessionCredentialSummary[]>("set_session_credential", { input }),
      ),
    activateSessionCredential: async (input) =>
      toSafeSessionCredentialSummaries(
        await request<SessionCredentialSummary[]>("activate_session_credential", { input }),
      ),
    reorderSessionCredentials: async (input) =>
      toSafeSessionCredentialSummaries(
        await request<SessionCredentialSummary[]>("reorder_session_credentials", { input }),
      ),
    removeSessionCredential: async (input) =>
      toSafeSessionCredentialSummaries(
        await request<SessionCredentialSummary[]>("remove_session_credential", { input }),
      ),
    testProviderConnection: (input) => request("test_provider_connection", { input }),
    subscribeToRunEvents: (listener) => {
      listeners.add(listener);
      return (() => listeners.delete(listener)) as Unlisten;
    },
  };
}

export type {
  CompareRunsResult,
  ContextBranchView,
  ContextCheckpointProvenance,
  ContextCheckpointProviderSnapshot,
  ContextCheckpointView,
  ContextCursor,
  ContextPreview,
  ContextTreeProjection,
  ProviderModelInfo,
  ProviderProfile,
  ProviderTemplate,
  RouteProjection,
  RunEvent,
  RunSnapshot,
  SessionCredentialSummary,
  WorkspaceDetail,
  WorkspaceSummary,
};
