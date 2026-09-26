export interface AgentProject {
  id: string;
  name: string;
  path: string;
}

export interface AgentEnvironment {
  available: boolean;
  running: boolean;
  orcaVersion: string | null;
  ompVersion: string | null;
  runtimeId: string | null;
  projects: AgentProject[];
  message: string | null;
}

export interface AgentMission {
  id: string;
  workspaceId: string;
  repositoryId: string;
  repositoryPath: string;
  objective: string;
  runId: string | null;
  coordinatorHandle: string | null;
  runtimeId: string | null;
  status: "creating" | "ready" | "needs-attention";
  error: string | null;
  createdAt: string;
}

export interface AgentTask {
  id: string;
  title: string;
  spec: string;
  status: string;
  dispatchId: string | null;
  terminalState: string | null;
  liveness: string | null;
  attention: string | null;
  canRelease: boolean;
}

export interface AgentMessage {
  id: string;
  type: string;
  body: string;
  taskId: string | null;
  dispatchId: string | null;
  requiresReply: boolean;
}

export interface AgentSnapshot {
  mission: AgentMission;
  tasks: AgentTask[];
  messages: AgentMessage[];
  connected: boolean;
  warning: string | null;
  canStartTasks?: boolean;
}

export interface AgentOperation {
  id: string;
  missionId: string;
  kind: string;
  status: "pending" | "succeeded" | "failed" | "unknown";
  receipt: unknown;
  error: string | null;
  createdAt: string;
}

export interface AgentOutput {
  text: string;
  cursor: string | null;
  hasMore: boolean;
  source: string | null;
  warning?: string | null;
}

export interface CreateAgentMissionInput {
  id: string;
  workspaceId: string;
  repositoryId: string;
  objective: string;
}

export interface StartAgentTaskInput {
  operationId: string;
  missionId: string;
  title: string;
  spec: string;
}

export interface ReplyToAgentInput {
  operationId: string;
  missionId: string;
  messageId: string;
  body: string;
}

export interface ReleaseAgentWorkerInput {
  operationId: string;
  missionId: string;
  dispatchId: string;
}

export interface ReconnectAgentMissionInput {
  operationId: string;
  missionId: string;
}

export interface ReadAgentOutputInput {
  missionId: string;
  dispatchId: string;
  cursor?: string;
}
