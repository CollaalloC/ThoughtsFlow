import { useCallback, useEffect, useRef, useState } from "react";
import type { DesktopBridge } from "../../platform/desktop-bridge";
import type {
  AgentEnvironment,
  AgentMessage,
  AgentMission,
  AgentOperation,
  AgentOutput,
  AgentSnapshot,
} from "../../shared/contracts";
import "./agent-workspace.css";

interface AgentWorkspaceProps {
  bridge: DesktopBridge;
  workspaceId?: string;
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

export function AgentWorkspace(props: AgentWorkspaceProps) {
  return <AgentWorkspaceContent key={props.workspaceId ?? "no-workspace"} {...props} />;
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
        <h2>把目标交给协作中的 Agent</h2>
        <p>Orca 管理并行工作区，OMP 执行任务。每个任务可修改自己的独立 worktree，成果由你审查后合并。</p>
      </header>
      {!workspaceId ? <p className="agent-workspace__notice">请先在 Focus 创建或打开工作区。</p> : null}
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
  const [title, setTitle] = useState("");
  const [spec, setSpec] = useState("");
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
  const titleError = byteLimitError("任务标题", title, 200);
  const specError = byteLimitError("任务说明", spec, 64_000);
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
      <form className="agent-workspace__panel" onSubmit={(event) => {
        event.preventDefault();
        if (!title.trim() || !spec.trim() || titleError || specError) return;
        void mutate("start-task", (operationId) => bridge.startAgentTask({ operationId, missionId: mission.id, title: title.trim(), spec: spec.trim() }), () => { setTitle(""); setSpec(""); });
      }}>
        <h4>分派一个任务</h4>
        <p>使用本机 OMP 的模型与认证设置。请在任务说明中写清范围、背景和验收标准。</p>
        <label>任务标题<input aria-label="任务标题" aria-invalid={!!titleError} aria-describedby={titleError ? "agent-title-error" : undefined} required value={title} onChange={(event) => setTitle(event.target.value)} disabled={busy} /></label>
        {titleError ? <p id="agent-title-error" className="agent-workspace__error" role="alert">{titleError}</p> : null}
        <label>任务说明<textarea aria-label="任务说明" aria-invalid={!!specError} aria-describedby={specError ? "agent-spec-error" : undefined} required rows={4} value={spec} onChange={(event) => setSpec(event.target.value)} placeholder="这个 Agent 应修改哪些内容、需要遵守什么约束、如何验证完成" disabled={busy} /></label>
        {specError ? <p id="agent-spec-error" className="agent-workspace__error" role="alert">{specError}</p> : null}
        <p>启动后，OMP 可在独立 worktree 中读写文件；不会自动合并，也不会使用 ThoughsFlow 的 Provider 凭据。</p>
        <button className="agent-workspace__primary" disabled={!canMutate || snapshot?.canStartTasks === false || uncertainStart || !title.trim() || !spec.trim() || !!titleError || !!specError} type="submit">{busy ? "正在提交…" : "启动 OMP 任务"}</button>
      </form>
      <section className="agent-workspace__tasks" aria-label="协作任务">
        <h4>任务进展 <span>{snapshot?.tasks.length ?? 0}</span></h4>
        {!snapshot?.tasks.length ? <p>还没有任务。把可以独立完成的工作分别交给 Agent。</p> : null}
        {snapshot?.tasks.map((task) => (
          <article className="agent-workspace__panel" key={task.id} aria-label={`任务 ${task.title}`}>
            <header className="agent-workspace__mission-header"><h4>{task.title}</h4><span className="agent-workspace__badge">{statusLabel(task.status)}</span></header>
            <p>{task.liveness ? `Agent ${statusLabel(task.liveness)}` : "Agent 存活状态未知"}{task.terminalState ? ` · ${statusLabel(task.terminalState)}` : ""}</p>
            {task.attention ? <p className="agent-workspace__notice">{attentionLabel(task.attention)}</p> : null}
            <details><summary>任务说明</summary><p className="agent-workspace__text">{task.spec}</p></details>
            {task.dispatchId ? <AgentTaskOutput key={task.dispatchId} bridge={bridge} missionId={mission.id} dispatchId={task.dispatchId} /> : null}
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

function AgentTaskOutput({ bridge, missionId, dispatchId }: { bridge: DesktopBridge; missionId: string; dispatchId: string }) {
  const [output, setOutput] = useState<AgentOutput | null>(null);
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
      const next = await bridge.readAgentOutput({ missionId, dispatchId, ...(append && output?.cursor ? { cursor: output.cursor } : {}) });
      if (active.current) setOutput((previous) => ({ ...next, text: append ? (previous?.text ?? "") + next.text : next.text }));
    } catch (reason) {
      if (active.current) setError(errorMessage(reason));
    } finally {
      inFlight.current = false;
      if (active.current) setBusy(false);
    }
  };
  return <details className="agent-workspace__output">
    <summary>Agent 输出</summary>
    <button disabled={busy} onClick={() => void read(false)} type="button">{busy ? "读取中…" : output ? "刷新输出" : "读取输出"}</button>
    {error ? <p className="agent-workspace__error" role="alert">{error}</p> : null}
    {output ? <pre>{output.text || "暂无输出。"}</pre> : null}
    {output?.warning ? <p className="agent-workspace__notice">{output.warning}</p> : null}
    {output?.hasMore && output.cursor ? <button disabled={busy} onClick={() => void read(true)} type="button">读取后续输出</button> : null}
  </details>;
}
