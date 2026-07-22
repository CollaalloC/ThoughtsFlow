import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  ApiEnvelope,
  CompareRunsResult,
  ContextPreview,
  CreateTurnAndStartRunInput,
  DecisionMark,
  DecisionStatus,
  ExportResult,
  InspectContextInput,
  ProviderProfile,
  RetryRunInput,
  RouteProjection,
  RunEvent,
  RunHandle,
  RunSnapshot,
  WorkspaceDetail,
  WorkspaceSummary,
} from "../shared/contracts";

export interface DesktopBridge {
  listWorkspaces(): Promise<WorkspaceSummary[]>;
  createWorkspace(input: { name: string; goal: string }): Promise<WorkspaceSummary>;
  openWorkspace(id: string): Promise<WorkspaceDetail>;
  updateWorkspace(input: {
    id: string;
    name?: string;
    goal?: string;
    archived?: boolean;
  }): Promise<WorkspaceSummary>;
  inspectContext(input: InspectContextInput): Promise<ContextPreview>;
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
  exportDecisionPacket(input: {
    workspaceId: string;
    destination?: string;
  }): Promise<ExportResult>;
  listProviderProfiles(): Promise<ProviderProfile[]>;
  saveProviderProfile(
    input: Omit<ProviderProfile, "id"> & { id?: string },
  ): Promise<ProviderProfile>;
  setSessionCredential(input: {
    providerProfileId: string;
    credential: string;
  }): Promise<void>;
  testProviderConnection(input: {
    providerProfileId: string;
  }): Promise<{ ok: boolean; message: string }>;
  subscribeToRunEvents(listener: (event: RunEvent) => void): () => void;
}

type Unlisten = () => void;

export function createDesktopBridge(): DesktopBridge {
  const listeners = new Set<(event: RunEvent) => void>();
  const dispatch = (event: RunEvent) => listeners.forEach((listener) => listener(event));
  const request = async <T>(command: string, args?: Record<string, unknown>) => {
    const response = await invoke<ApiEnvelope<T>>(command, args);
    if (response.apiVersion !== 1) {
      throw new Error(`Unsupported DesktopBridge API version: ${response.apiVersion}`);
    }
    return response.data;
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
    listWorkspaces: () => request("list_workspaces"),
    createWorkspace: (input) => request("create_workspace", { input }),
    openWorkspace: (id) => request("open_workspace", { id }),
    updateWorkspace: (input) => request("update_workspace", { input }),
    inspectContext: (input) => request("inspect_context", { input }),
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
    listProviderProfiles: () => request("list_provider_profiles"),
    saveProviderProfile: (input) => request("save_provider_profile", { input }),
    setSessionCredential: (input) => request("set_session_credential", { input }),
    testProviderConnection: (input) => request("test_provider_connection", { input }),
    subscribeToRunEvents: (listener) => {
      listeners.add(listener);
      return (() => listeners.delete(listener)) as Unlisten;
    },
  };
}

export type {
  CompareRunsResult,
  ContextPreview,
  ProviderProfile,
  RouteProjection,
  RunEvent,
  RunSnapshot,
  WorkspaceDetail,
  WorkspaceSummary,
};
