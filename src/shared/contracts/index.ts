export type RunStatus =
  | "pending"
  | "connecting"
  | "streaming"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted";

export type DecisionStatus = "accepted" | "rejected" | "to-verify";
export type ProviderDialect =
  | "openai-compatible"
  | "ollama"
  | "anthropic"
  | "google-generative-ai";
export type ProviderStreamProtocol =
  | "openai_sse"
  | "ollama_ndjson"
  | "anthropic_sse"
  | "google_sse";
export type ProviderAuthPlacement =
  | "none"
  | "bearer_header"
  | "api_key_header"
  | "query_param";

export interface WorkspaceSummary {
  id: string;
  name: string;
  goal: string;
  systemPrompt: string;
  archived: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ModelRun {
  id: string;
  turnId: string;
  status: RunStatus;
  output: string;
  reasoning?: string | null;
  providerProfileId: string;
  providerName: string;
  model: string;
  baseUrl: string;
  createdAt: string;
  completedAt?: string | null;
  usage?: Record<string, number> | null;
  error?: { code: string; message: string; retryable?: boolean; status?: number | null } | null;
}

export interface Turn {
  id: string;
  workspaceId: string;
  parentRunId: string | null;
  prompt: string;
  title?: string;
  createdAt: string;
  runs: ModelRun[];
}

export interface DecisionMark {
  id: string;
  workspaceId: string;
  runId: string;
  status: DecisionStatus;
  reason: string;
  createdAt: string;
}

export interface WorkspaceDetail {
  workspace: WorkspaceSummary;
  turns: Turn[];
  selectedRunIds: Record<string, string>;
  adjacentBranches: Array<{ runId: string; label: string }>;
  decisionMarks: DecisionMark[];
}

export interface ContextItem {
  id: string;
  ordinal: number;
  role: "system" | "user" | "assistant";
  label: string;
  source: string;
  content: string;
  reason: string;
  estimatedTokens: number;
  included: boolean;
  pinned: boolean;
}

export interface ContextPreview {
  hash: string;
  estimatedTokens: number;
  limitTokens: number;
  blocked: boolean;
  warnings: string[];
  providerProfileId: string;
  providerName: string;
  model: string;
  baseUrl: string;
  items: ContextItem[];
}

export interface RunSnapshot {
  id: string;
  runId: string;
  canonicalHash: string;
  createdAt: string;
  providerId?: string;
  templateRevision?: number;
  providerName: string;
  streamProtocol?: ProviderStreamProtocol;
  authPlacement?: ProviderAuthPlacement;
  authHeaderName?: string;
  additionalHeaders: Record<string, string>;
  model: string;
  baseUrl: string;
  parameters: Record<string, unknown>;
  items: ContextItem[];
}

export interface ProviderProfile {
  id: string;
  providerId: string;
  name: string;
  dialect: ProviderDialect;
  baseUrl: string;
  model: string;
  isDefault: boolean;
  parameters?: Record<string, unknown>;
}

export interface SessionCredentialSummary {
  credentialId: string;
  label: string;
  order: number;
  isActive: boolean;
}

export interface ProtocolProfile {
  streamProtocol: ProviderStreamProtocol;
  authPlacement: ProviderAuthPlacement;
  authHeaderName?: string;
  modelsEndpoint?: string;
  requiresAdditionalHeaders: boolean;
  additionalHeaders: Record<string, string>;
}

export interface ProviderTemplate {
  providerId: string;
  revision: number;
  displayName: string;
  defaultBaseUrl: string;
  protocol: ProtocolProfile;
  runtimeAvailable: boolean;
}

export interface ProviderModelInfo {
  id: string;
  displayName: string;
  contextWindow: number | null;
  supportsTools: boolean | null;
}

export type ListProviderModelsInput =
  | {
      providerProfileId: string;
      draft?: never;
    }
  | {
      providerProfileId?: never;
      draft: {
        providerId: string;
        baseUrl: string;
        sessionCredential?: string;
      };
    };

export interface SaveProviderProfileInput {
  id?: string;
  providerId: string;
  name: string;
  baseUrl: string;
  model: string;
  isDefault: boolean;
  parameters?: Record<string, unknown>;
}

export interface RunEvent {
  apiVersion: 1;
  type:
    | "run-started"
    | "text-delta"
    | "reasoning-delta"
    | "usage-updated"
    | "checkpoint-saved"
    | "provider-metadata"
    | "run-completed"
    | "run-failed"
    | "run-cancelled"
    | "persistence-failed";
  runId: string;
  text?: string;
  usage?: Record<string, number>;
  metadata?: Record<string, unknown>;
  error?: { code: string; message: string; retryable?: boolean; status?: number | null };
  at: string;
}

export interface ApiEnvelope<T> {
  apiVersion: 1;
  data: T;
}

export interface InspectContextInput {
  workspaceId: string;
  parentRunId: string | null;
  prompt: string;
  providerProfileId: string;
}

export interface CreateTurnAndStartRunInput extends InspectContextInput {
  previewHash: string;
}

export interface RetryRunInput {
  runId: string;
  providerProfileId: string;
  previewHash: string;
  credentialId?: string;
}

export interface RunHandle {
  turnId: string;
  runId: string;
}

export interface RouteProjection {
  workspaceId: string;
  nodes: Array<{
    id: string;
    turnId: string;
    title: string;
    summary: string;
    status: RunStatus;
    x: number;
    y: number;
    isCurrent: boolean;
    isOnCurrentLineage: boolean;
    runs: Array<{
      runId: string;
      label: string;
      model: string;
      status: RunStatus;
      canBranch: boolean;
    }>;
  }>;
  edges: Array<{
    id: string;
    sourceRunId: string;
    targetTurnId: string;
    isOnCurrentLineage: boolean;
  }>;
}

export interface CompareRunsResult {
  left: { runId: string; model: string; status: RunStatus };
  right: { runId: string; model: string; status: RunStatus };
  answer: { leftMarkdown: string; rightMarkdown: string };
  contextDiff: {
    onlyLeft: ContextDiffItem[];
    onlyRight: ContextDiffItem[];
    shared: ContextDiffItem[];
  };
}

export interface ContextDiffItem {
  id: string;
  ordinal: number;
  role: ContextItem["role"];
  source: string;
  preview: string;
}

export interface ExportResult {
  path: string;
  bytesWritten: number;
}
