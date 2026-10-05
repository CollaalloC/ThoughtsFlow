import { useCallback, useEffect, useRef, useState } from "react";
import { DesktopBridgeError, type DesktopBridge } from "../../platform/desktop-bridge";
import type {
  AgentEnvironment,
  AgentEvidence,
  WorkspaceSummary,
  AgentMessage,
  AgentMission,
  AgentOperation,
  AgentOutput,
  AgentSnapshot,
} from "../../shared/contracts";
import { AgentHandoffPanel } from "./AgentHandoffPanel";
import "./agent-workspace.css";

interface AgentWorkspaceProps {
  bridge: DesktopBridge;
  workspaceId?: string;
  onWorkspaceChange?: (id: string) => void;
}

function errorMessage(reason: unknown) {
  return reason instanceof Error ? reason.message : "无法连接本机 Agent 运行环境。";
}

function byteLimitError(label: string, value: string, limit: number) {
  const bytes = new TextEncoder().encode(value.trim()).length;
  return bytes > limit ? `${label}超过 ${limit} 字节上限（当前 ${bytes} 字节），请缩短后再提交。` : null;
}

function documentVisible() {
  return document.visibilityState !== "hidden";
}

const statusLabels: Record<string, string> = {
  pending: "等待确认", succeeded: "已执行", failed: "失败", unknown: "结果未知",
  creating: "准备中", ready: "就绪", "needs-attention": "需要处理",
  queued: "排队中", running: "进行中", completed: "已完成", cancelled: "已取消",
  blocked: "等待处理", alive: "在线", idle: "空闲", busy: "工作中",
  exited: "已退出", dead: "已停止", released: "已释放", stopped: "已停止",
  live: "在线", unverifiable: "暂时无法确认", reclaimable: "可释放", retained: "已保留",
  release_pending: "正在释放", release_unknown: "释放结果未确认",
  dispatched: "已派发", in_progress: "进行中", outcome_unknown: "执行结果未确认",
  finished_unverified: "执行已结束，结果待核实",
};

const attentionLabels: Record<string, string> = {
  guidance: "Agent 需要你的指导。",
  input: "Agent 正等待补充信息。",
  approval: "有操作等待批准，请在 Orca 中处理。",
  failure: "任务执行失败，请检查输出。",
  interruption: "任务已中断，请检查输出与当前状态。",
  stale: "进展较长时间未更新，请检查 Orca 中的 Agent。",
  unverifiable: "暂时无法确认执行状态，请在 Orca 中检查。",
  root_completion: "任务已完成，请审查成果。",
};

function attentionLabel(attention: string) {
  return attention.split(",").map((category) => {
    const detail = category.trim();
    return attentionLabels[detail] ?? `请在 Orca 中检查：${detail}`;
  }).join(" ");
}

function statusLabel(status: string) {
  return statusLabels[status] ?? status;
}

export function AgentWorkspace({ bridge, workspaceId, onWorkspaceChange }: AgentWorkspaceProps) {
  const [workspaces, setWorkspaces] = useState<WorkspaceSummary[]>([]);
  const [selectedId, setSelectedId] = useState(workspaceId);
  const [name, setName] = useState("");
  const [goal, setGoal] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const alive = useRef(true);
  const inFlight = useRef(false);
  const currentId = workspaceId ?? selectedId;
  const selection = useRef(currentId);
  selection.current = currentId;
  const changeCallback = useRef(onWorkspaceChange);
  changeCallback.current = onWorkspaceChange;
  useEffect(() => {
    alive.current = true;
    let current = true;
    void bridge.listWorkspaces().then((items) => {
      if (!current) return;
      setWorkspaces(items);
      if (!selection.current) {
        const first = items.find((item) => !item.archived)?.id;
        if (first) { setSelectedId(first); changeCallback.current?.(first); }
      }
    }).catch((reason: unknown) => { if (current) setError(errorMessage(reason)); });
    return () => { current = false; alive.current = false; };
  }, [bridge]);
  const choose = (id: string) => { setSelectedId(id); onWorkspaceChange?.(id); };
  const create = async () => {
    if (inFlight.current || !name.trim()) return;
    inFlight.current = true;
    setCreating(true);
    setError(null);
    try {
      const next = await bridge.createWorkspace({ name: name.trim(), goal: goal.trim() });
      if (!alive.current) return;
      setWorkspaces((items) => [next, ...items.filter((item) => item.id !== next.id)]);
      setName(""); setGoal(""); choose(next.id);
    } catch (reason) { if (alive.current) setError(errorMessage(reason)); }
    finally { inFlight.current = false; if (alive.current) setCreating(false); }
  };
  return <section aria-label="Agent 工作台">
    <div className="agent-workspace agent-workspace__workspace-picker">
      <label>本地工作区<select aria-label="本地工作区" value={currentId ?? ""} disabled={creating} onChange={(event) => choose(event.target.value)}>
        <option value="">选择工作区</option>
        {currentId && !workspaces.some((item) => item.id === currentId) ? <option value={currentId}>{currentId}</option> : null}
        {workspaces.filter((item) => !item.archived).map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
      </select></label>
      <details open={!currentId}><summary>创建本地工作区</summary><form onSubmit={(event) => { event.preventDefault(); void create(); }}>
        <label>工作区名称<input aria-label="工作区名称" required value={name} onChange={(event) => setName(event.target.value)} disabled={creating} /></label>
        <label>工作区目标<input aria-label="工作区目标" value={goal} onChange={(event) => setGoal(event.target.value)} disabled={creating} /></label>
        <button type="submit" disabled={creating || !name.trim()}>{creating ? "正在创建工作区…" : "创建工作区"}</button>
      </form></details>
      <p>工作区保存上下文与任务证据，无需配置 Provider。</p>
      {error ? <p role="alert" className="agent-workspace__error">{error}</p> : null}
    </div>
    <AgentWorkspaceContent key={currentId ?? "no-workspace"} bridge={bridge} workspaceId={currentId} />
  </section>;
}

function AgentWorkspaceContent({ bridge, workspaceId }: AgentWorkspaceProps) {
  const [environment, setEnvironment] = useState<AgentEnvironment | null>(null);
  const [missions, setMissions] = useState<AgentMission[]>([]);
  const [missionId, setMissionId] = useState("");
  const [repositoryId, setRepositoryId] = useState("");
  const [objective, setObjective] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const busyRef = useRef(false);
  const active = useRef(true);
  const objectiveError = byteLimitError("协作目标", objective, 32_000);

  useEffect(() => {
    active.current = true;
    let current = true;
    void Promise.allSettled([
      bridge.agentEnvironment(),
      workspaceId ? bridge.listAgentMissions(workspaceId) : Promise.resolve([]),
    ]).then(([runtime, history]) => {
      if (!current) return;
      if (runtime.status === "fulfilled") setEnvironment(runtime.value);
      if (history.status === "fulfilled") {
        setMissions(history.value);
        setMissionId(history.value[0]?.id ?? "");
      }
      const failures = [runtime, history].filter((result) => result.status === "rejected");
      if (failures.length) setError(failures.map((result) => errorMessage(result.reason)).join(" "));
    });
    return () => { current = false; active.current = false; };
  }, [bridge, workspaceId]);

  const inspectEnvironment = async (open: boolean) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy("environment");
    setError(null);
    try {
      const result = await (open ? bridge.openAgentRuntime() : bridge.agentEnvironment());
      if (active.current) setEnvironment(result);
    } catch (reason) {
      if (active.current) setError(errorMessage(reason));
    } finally {
      busyRef.current = false;
      if (active.current) setBusy(null);
    }
  };

  const createMission = async () => {
    if (busyRef.current || !workspaceId || !repositoryId || !objective.trim() || objectiveError) return;
    busyRef.current = true;
    setBusy("create");
    setError(null);
    try {
      const mission = await bridge.createAgentMission({
        id: crypto.randomUUID(), workspaceId, repositoryId, objective: objective.trim(),
      });
      if (!active.current) return;
      setMissions((previous) => [mission, ...previous.filter((item) => item.id !== mission.id)]);
      setMissionId(mission.id);
      setObjective("");
    } catch (reason) {
      if (!active.current) return;
      setError(`${errorMessage(reason)} 请先检查历史协作与操作回执，确认结果后再创建。`);
      try {
        const history = await bridge.listAgentMissions(workspaceId);
        if (active.current) {
          setMissions(history);
          setMissionId((previous) => previous || history[0]?.id || "");
        }
      } catch { /* Preserve the original error and the current mission list. */ }
    } finally {
      busyRef.current = false;
      if (active.current) setBusy(null);
    }
  };

  const selectedMission = missions.find((mission) => mission.id === missionId);
  const runtimeReady = environment?.available && environment.running && !!environment.ompVersion;

  return (
    <section className="agent-workspace" aria-label="Agent 协作工作面">
      <header className="agent-workspace__header">
        <span className="agent-workspace__eyebrow">AGENTS</span>
        <h2>管理上下文，编排现有 Agent</h2>
        <p>ThoughtsFlow 管理图谱、交接内容与工作流证据；Orca 管理独立工作区，OMP 沿用已有 harness 执行任务。</p>
      </header>
      {!workspaceId ? <p className="agent-workspace__notice">请先在上方创建或选择本地工作区。</p> : null}
      <section className="agent-workspace__runtime" aria-label="运行环境">
        <div>
          <strong>本机运行环境</strong>
          <p>{environment
            ? `Orca ${environment.orcaVersion ?? "未检测到"} · ${environment.running ? "运行中" : "未连接"} · OMP ${environment.ompVersion ?? "未检测到"}`
            : "正在检测 Orca 与 OMP…"}</p>
          {environment?.message ? <p>{environment.message}</p> : null}
        </div>
        <div className="agent-workspace__actions">
          <button disabled={!!busy} onClick={() => void inspectEnvironment(false)} type="button">重新检测</button>
          {environment && !environment.running ? (
            <button disabled={!!busy || !environment.available} onClick={() => void inspectEnvironment(true)} type="button">启动 Orca</button>
          ) : null}
        </div>
      </section>
      {error ? <p className="agent-workspace__error" role="alert">{error}</p> : null}
      <div className="agent-workspace__layout">
        <aside className="agent-workspace__sidebar">
          <label>
            历史协作
            <select aria-label="历史协作" value={missionId} onChange={(event) => setMissionId(event.target.value)} disabled={!!busy}>
              <option value="">选择协作</option>
              {missions.map((mission) => <option key={mission.id} value={mission.id}>{mission.objective}</option>)}
            </select>
          </label>
          <form className="agent-workspace__panel" onSubmit={(event) => { event.preventDefault(); void createMission(); }}>
            <h3>新建协作</h3>
            <label>
              代码仓库
              <select aria-label="代码仓库" required value={repositoryId} onChange={(event) => setRepositoryId(event.target.value)} disabled={!!busy || !runtimeReady || !workspaceId}>
                <option value="">选择 Orca 中的仓库</option>
                {environment?.projects.map((project) => <option key={project.id} value={project.id}>{project.name} · {project.path}</option>)}
              </select>
            </label>
            {environment?.running && !environment.projects.length ? <p>请先在 Orca 中登记代码仓库，再重新检测。</p> : null}
            <label>
              协作目标
              <textarea aria-label="协作目标" aria-invalid={!!objectiveError} aria-describedby={objectiveError ? "agent-objective-error" : undefined} required rows={4} value={objective} onChange={(event) => setObjective(event.target.value)} placeholder="说明要完成的目标与验收标准" disabled={!!busy || !workspaceId} />
            </label>
            {objectiveError ? <p id="agent-objective-error" className="agent-workspace__error" role="alert">{objectiveError}</p> : null}
            <button className="agent-workspace__primary" disabled={!!busy || !runtimeReady || !workspaceId || !repositoryId || !objective.trim() || !!objectiveError} type="submit">
              {busy === "create" ? "正在创建…" : "创建协作"}
            </button>
          </form>
        </aside>
        {selectedMission ? (
          <AgentMissionView key={selectedMission.id} bridge={bridge} mission={selectedMission} />
        ) : <div className="agent-workspace__empty">选择已有协作，或选定仓库开始一个新目标。</div>}
      </div>
    </section>
  );
}

function AgentMissionView({ bridge, mission }: { bridge: DesktopBridge; mission: AgentMission }) {
  const [snapshot, setSnapshot] = useState<AgentSnapshot | null>(null);
  const [operations, setOperations] = useState<AgentOperation[]>([]);
  const [receiptsLoaded, setReceiptsLoaded] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [evidenceRevision, setEvidenceRevision] = useState(0);
  const [answered, setAnswered] = useState<string[]>([]);
  const [uncertainTargets, setUncertainTargets] = useState<string[]>([]);
  const [visible, setVisible] = useState(documentVisible);
  const [needsFreshRead, setNeedsFreshRead] = useState(true);
  const active = useRef(true);
  const readInFlight = useRef(false);
  const refreshQueued = useRef(false);
  const latestRefresh = useRef<() => Promise<void>>(async () => {});
  const mutationInFlight = useRef(false);
  const revision = useRef(0);
  const connected = snapshot?.connected === true && !readError && !needsFreshRead;
  const uncertain = operations.some((operation) => operation.status === "pending" || operation.status === "unknown");
  const uncertainStart = operations.some((operation) =>
    (operation.status === "pending" || operation.status === "unknown") &&
    ["create-mission", "start-task"].includes(operation.kind),
  );
  const canMutate = connected && receiptsLoaded && !busy && visible;
  const canReconnect = !busy && visible && !needsFreshRead;
  const missionWarning = snapshot ? snapshot.warning ?? snapshot.mission.error : mission.error;

  const refresh = useCallback(async () => {
    if (!active.current) return;
    // Coalesce explicit refreshes; polling never queues behind reads or commands.
    if (readInFlight.current || mutationInFlight.current || !documentVisible()) {
      refreshQueued.current = true;
      return;
    }
    refreshQueued.current = false;
    readInFlight.current = true;
    const readRevision = revision.current;
    try {
      const [nextSnapshot, nextOperations] = await Promise.allSettled([
        bridge.getAgentSnapshot(mission.id), bridge.listAgentOperations(mission.id),
      ]);
      if (!active.current || readRevision !== revision.current) return;
      if (nextSnapshot.status === "fulfilled") {
        const next = nextSnapshot.value;
        setSnapshot((previous) => !next.connected && previous
          ? { ...next, tasks: previous.tasks, messages: previous.messages }
          : next);
      }
      if (nextOperations.status === "fulfilled") {
        setOperations((previous) => [
          ...nextOperations.value,
          ...previous.filter((operation) => !nextOperations.value.some((item) => item.id === operation.id)),
        ]);
        setReceiptsLoaded(true);
      }
      const failures = [nextSnapshot, nextOperations].filter((result) => result.status === "rejected");
      setReadError(failures.length ? failures.map((result) => errorMessage(result.reason)).join(" ") : null);
      setNeedsFreshRead(false);
    } finally {
      readInFlight.current = false;
      if (active.current && refreshQueued.current && !mutationInFlight.current && documentVisible()) {
        void latestRefresh.current();
      }
    }
  }, [bridge, mission.id]);
  latestRefresh.current = refresh;

  useEffect(() => {
    active.current = true;
    const onVisibilityChange = () => {
      revision.current += 1;
      setVisible(documentVisible());
      setNeedsFreshRead(true);
      if (documentVisible()) void refresh();
    };
    void refresh();
    const timer = window.setInterval(() => {
      if (documentVisible() && !readInFlight.current && !mutationInFlight.current) void refresh();
    }, 5_000);
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      active.current = false;
      revision.current += 1;
      refreshQueued.current = false;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [refresh]);

  const mutate = async (kind: string, send: (operationId: string) => Promise<AgentOperation>, onSuccess?: () => void, target?: string) => {
    if (mutationInFlight.current || (kind === "reconnect" ? !canReconnect : !canMutate) || (kind === "start-task" && (uncertainStart || snapshot?.canStartTasks === false)) || (target && uncertainTargets.includes(target))) return;
    mutationInFlight.current = true;
    revision.current += 1;
    setNeedsFreshRead(true);
    setBusy(true);
    setNotice(null);
    const operationId = crypto.randomUUID();
    let operation: AgentOperation;
    try {
      operation = await send(operationId);
    } catch (reason) {
      if (kind === "start-task" && reason instanceof DesktopBridgeError && ["agent_handoff_stale", "agent_handoff_required", "agent_handoff"].includes(reason.code)) {
        mutationInFlight.current = false;
        revision.current += 1;
        if (active.current) { setBusy(false); void refresh(); }
        throw reason;
      }
      operation = {
        id: operationId, missionId: mission.id, kind, status: "unknown", receipt: null,
        error: errorMessage(reason), createdAt: new Date().toISOString(),
      };
    }
    mutationInFlight.current = false;
    if (!active.current) return;
    revision.current += 1;
    setOperations((previous) => [operation, ...previous.filter((item) => item.id !== operation.id)]);
    setBusy(false);
    if (operation.status === "succeeded") {
      setNotice(kind === "start-task" ? "任务已提交。执行进展会自动更新。" : "操作已执行，正在更新状态。");
      onSuccess?.();
    } else if (operation.status === "failed") {
      setNotice(`操作失败：${operation.error ?? "请查看操作回执。"}`);
    } else {
      if (target) setUncertainTargets((previous) => [...previous, target]);
      setNotice("操作结果尚未确认，请勿重复提交。请检查操作回执和 Orca 中的实际任务。");
    }
    void refresh();
    return operation;
  };

  return (
    <section className="agent-workspace__mission" aria-label="当前协作">
      <header className="agent-workspace__mission-header">
        <div><h3>{mission.objective}</h3><p>{mission.repositoryPath}</p></div>
        <button onClick={() => void refresh()} type="button">刷新进展</button>
      </header>
      <p className={connected ? "agent-workspace__connected" : "agent-workspace__notice"} role="status">
        {!visible ? "窗口在后台 · 已暂停自动刷新" : needsFreshRead && snapshot ? "正在更新进展 · 保留最近一次进展" : connected ? "已连接 · 每 5 秒更新进展" : snapshot || readError ? "连接已断开 · 保留最近一次进展" : "正在连接协作…"}
      </p>
      {readError ? <p className="agent-workspace__error" role="alert">{readError}</p> : null}
      {missionWarning ? <p className="agent-workspace__notice">{missionWarning}</p> : null}
      {snapshot && !snapshot.connected ? (
        <button disabled={!canReconnect} onClick={() => void mutate("reconnect", (operationId) => bridge.reconnectAgentMission({ operationId, missionId: mission.id }))} type="button">重新连接协作</button>
      ) : null}
      {notice ? <p className="agent-workspace__notice" role="status">{notice}</p> : null}
      {uncertain ? <p className="agent-workspace__notice">有操作尚未确认，请查看下方回执并核对 Orca。{uncertainStart ? "已暂停新增任务，避免重复启动。" : "请勿重复提交同一操作。"}</p> : null}
      <AgentHandoffPanel bridge={bridge} mission={mission} busy={busy}
        canStart={canMutate && snapshot?.canStartTasks !== false && !uncertainStart}
        evidenceRevision={evidenceRevision}
        onStart={(input) => mutate("start-task", (operationId) => bridge.startAgentTask({ ...input, operationId }))} />
      <section className="agent-workspace__tasks" aria-label="协作任务">
        <h4>任务进展 <span>{snapshot?.tasks.length ?? 0}</span></h4>
        {!snapshot?.tasks.length ? <p>还没有任务。把可以独立完成的工作分别交给 Agent。</p> : null}
        {snapshot?.tasks.map((task) => (
          <article className="agent-workspace__panel" key={task.id} aria-label={`任务 ${task.title}`}>
            <header className="agent-workspace__mission-header"><h4>{task.title}</h4><span className="agent-workspace__badge">{statusLabel(task.status)}</span></header>
            <p>{task.liveness ? `Agent ${statusLabel(task.liveness)}` : "Agent 存活状态未知"}{task.terminalState ? ` · ${statusLabel(task.terminalState)}` : ""}</p>
            {task.attention ? <p className="agent-workspace__notice">{attentionLabel(task.attention)}</p> : null}
            <details><summary>任务说明</summary><p className="agent-workspace__text">{task.spec}</p></details>
            {task.dispatchId ? <AgentTaskOutput key={task.dispatchId} bridge={bridge} missionId={mission.id} dispatchId={task.dispatchId} onSaved={() => setEvidenceRevision((value) => value + 1)} /> : null}
            {task.canRelease && task.dispatchId ? <button disabled={!canMutate || uncertainTargets.includes(`release:${task.dispatchId}`)} onClick={() => void mutate("release", (operationId) => bridge.releaseAgentWorker({ operationId, missionId: mission.id, dispatchId: task.dispatchId! }), undefined, `release:${task.dispatchId}`)} type="button">释放已结束的 Agent</button> : null}
          </article>
        ))}
      </section>
      <section className="agent-workspace__messages" aria-label="协作消息">
        <h4>消息与待回复问题</h4>
        {!snapshot?.messages.length ? <p>暂无消息。</p> : null}
        {snapshot?.messages.map((message) => (
          <article className="agent-workspace__panel" key={message.id}>
            <p className="agent-workspace__text">{message.body}</p>
            {message.requiresReply ? <AgentReply message={message} disabled={!canMutate || answered.includes(message.id) || uncertainTargets.includes(`reply:${message.id}`)} onReply={(body) => void mutate("reply", (operationId) => bridge.replyToAgent({ operationId, missionId: mission.id, messageId: message.id, body }), () => setAnswered((previous) => [...previous, message.id]), `reply:${message.id}`)} /> : null}
          </article>
        ))}
      </section>
      <details className="agent-workspace__receipts">
        <summary>操作回执（{operations.length}）</summary>
        <p>回执用于确认操作结果与恢复协作。结果未知时，请先核对已有任务。</p>
        {operations.map((operation) => <article key={operation.id}><strong>{operation.kind} · {statusLabel(operation.status)}</strong><p>{operation.createdAt}</p><code>{operation.id}</code>{operation.error ? <p>{operation.error}</p> : null}<pre>{JSON.stringify(operation, null, 2)}</pre><button type="button" onClick={() => {
          const failed = () => setNotice("无法访问剪贴板，请在操作回执中手动选取并复制。");
          if (!navigator.clipboard) { failed(); return; }
          void navigator.clipboard.writeText(JSON.stringify(operation, null, 2)).catch(failed);
        }}>复制回执</button></article>)}
      </details>
    </section>
  );
}

function AgentReply({ message, disabled, onReply }: { message: AgentMessage; disabled: boolean; onReply: (body: string) => void }) {
  const [body, setBody] = useState("");
  const bodyError = byteLimitError("回复内容", body, 32_000);
  return <form onSubmit={(event) => { event.preventDefault(); if (!disabled && body.trim() && !bodyError) onReply(body.trim()); }}>
    <label>回复此问题<textarea aria-label={`回复问题 ${message.id}`} aria-invalid={!!bodyError} aria-describedby={bodyError ? `agent-reply-error-${message.id}` : undefined} required rows={2} value={body} onChange={(event) => setBody(event.target.value)} disabled={disabled} /></label>
    {bodyError ? <p id={`agent-reply-error-${message.id}`} className="agent-workspace__error" role="alert">{bodyError}</p> : null}
    <button disabled={disabled || !body.trim() || !!bodyError} type="submit">发送回复</button>
  </form>;
}

function AgentTaskOutput({ bridge, missionId, dispatchId, onSaved }: { bridge: DesktopBridge; missionId: string; dispatchId: string; onSaved: () => void }) {
  const [output, setOutput] = useState<AgentOutput | null>(null);
  const [saved, setSaved] = useState<AgentEvidence | null>(null);
  const pageCursor = useRef<string | undefined>(undefined);
  const captureId = useRef<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const active = useRef(true);
  const inFlight = useRef(false);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  const read = async (append: boolean) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setError(null);
    try {
      const cursor = append && output?.cursor ? output.cursor : undefined;
      const next = await bridge.readAgentOutput({ missionId, dispatchId, ...(cursor ? { cursor } : {}) });
      if (active.current) {
        pageCursor.current = cursor;
        captureId.current = null;
        setSaved(null);
        setOutput((previous) => ({ ...next, text: append ? (previous?.text ?? "") + next.text : next.text }));
      }
    } catch (reason) {
      if (active.current) setError(errorMessage(reason));
    } finally {
      inFlight.current = false;
      if (active.current) setBusy(false);
    }
  };
  const capture = async () => {
    if (inFlight.current || saved) return;
    inFlight.current = true;
    captureId.current ??= crypto.randomUUID();
    setBusy(true); setError(null);
    try {
      const next = await bridge.captureAgentEvidence({ id: captureId.current, missionId, dispatchId,
        ...(pageCursor.current ? { cursor: pageCursor.current } : {}) });
      if (active.current) { setSaved(next); onSaved(); }
    } catch (reason) { if (active.current) setError(`${errorMessage(reason)} 再次保存会沿用同一请求标识。`); }
    finally { inFlight.current = false; if (active.current) setBusy(false); }
  };
  return <details className="agent-workspace__output">
    <summary>Agent 输出</summary>
    <button disabled={busy} onClick={() => void read(false)} type="button">{busy ? "读取中…" : output ? "刷新输出" : "读取输出"}</button>
    {error ? <p className="agent-workspace__error" role="alert">{error}</p> : null}
    {output ? <pre>{output.text || "暂无输出。"}</pre> : null}
    {output?.warning ? <p className="agent-workspace__notice">{output.warning}</p> : null}
    {output ? <><p>保存会由后端重新读取当前页，记录 Dispatch、来源、游标与 hash；不代表完整回答。</p><button type="button" disabled={busy || !!saved} onClick={() => void capture()}>保存当前页观察（重新读取）</button></> : null}
    {saved ? <p role="status">输出观察已保存，可在下一任务中选择。{saved.hasMore ? " 还有后续页。" : ""}</p> : null}
    {output?.hasMore && output.cursor ? <button disabled={busy} onClick={() => void read(true)} type="button">读取后续输出</button> : null}
  </details>;
}
