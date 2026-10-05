import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ContextTree } from "../context-tree";
import type { DesktopBridge } from "../../platform/desktop-bridge";
import type { AgentEvidence, AgentHandoff, AgentHandoffPreview, AgentMission, AgentOperation, ContextTreeProjection, StartAgentTaskInput } from "../../shared/contracts";
import { AgentWorkflowGraph } from "./AgentWorkflowGraph";

interface Props {
  bridge: DesktopBridge;
  mission: AgentMission;
  busy: boolean;
  canStart: boolean;
  evidenceRevision: number;
  onStart: (input: Omit<StartAgentTaskInput, "operationId">) => Promise<AgentOperation | undefined>;
}

function message(reason: unknown) {
  return reason instanceof Error ? reason.message : "无法读取任务上下文。";
}

function byteLimit(label: string, value: string, limit: number) {
  const bytes = new TextEncoder().encode(value.trim()).length;
  return bytes > limit ? `${label}超过 ${limit} 字节上限（当前 ${bytes} 字节），请缩短后再提交。` : null;
}

export function AgentHandoffPanel({ bridge, mission, busy, canStart, evidenceRevision, onStart }: Props) {
  const [title, setTitle] = useState("");
  const [spec, setSpec] = useState("");
  const [includeActiveContext, setIncludeActiveContext] = useState(false);
  const [evidenceIds, setEvidenceIds] = useState<string[]>([]);
  const [evidence, setEvidence] = useState<AgentEvidence[]>([]);
  const [handoffs, setHandoffs] = useState<AgentHandoff[]>([]);
  const handoffTitles = useMemo(() => new Map(handoffs.map((item) => [item.operationId, item.preview.title?.trim()])), [handoffs]);
  const evidenceTitle = (item: AgentEvidence) => (item.handoffOperationId ? handoffTitles.get(item.handoffOperationId) : null) || item.taskId.slice(0, 12);
  const [tree, setTree] = useState<ContextTreeProjection | null>(null);
  const runTitles = useMemo(() => Object.fromEntries((tree?.nodes ?? []).map((node) => [node.runId, node.title?.trim() || node.prompt.trim().slice(0, 60)])), [tree]);
  const [preview, setPreview] = useState<AgentHandoffPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  const [reload, setReload] = useState(0);
  const alive = useRef(true);
  const revision = useRef(0);
  const contextRead = useRef(0);
  const inFlight = useRef(false);
  const titleError = byteLimit("任务标题", title, 200);
  const specError = byteLimit("任务说明", spec, 64_000);
  const valid = !!title.trim() && !!spec.trim() && !titleError && !specError;
  const invalidate = useCallback(() => { revision.current += 1; setPreview(null); setError(null); }, []);

  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; revision.current += 1; };
  }, []);

  useEffect(() => {
    let active = true;
    const request = ++contextRead.current;
    void Promise.allSettled([
      bridge.getContextTree({ workspaceId: mission.workspaceId }),
      bridge.listAgentEvidence(mission.workspaceId),
      bridge.listAgentHandoffs(mission.id),
    ]).then(([context, saved, history]) => {
      if (!active) return;
      if (context.status === "fulfilled" && request === contextRead.current) setTree(context.value);
      if (saved.status === "fulfilled") setEvidence(saved.value);
      if (history.status === "fulfilled") setHandoffs(history.value);
      const failures = [context, saved, history].filter((result) => result.status === "rejected");
      setHistoryError(failures.length ? failures.map((result) => message(result.reason)).join(" ") : null);
    });
    return () => { active = false; };
  }, [bridge, mission.id, mission.workspaceId, evidenceRevision, reload]);

  const selectContext = async (runId: string | null, branchId: string | null) => {
    if (!tree || inFlight.current || busy) return;
    invalidate();
    const request = ++contextRead.current;
    inFlight.current = true;
    setWorking(true);
    try {
      await bridge.setActiveContext({ workspaceId: mission.workspaceId, runId, branchId,
        expectedCursorVersion: tree.cursor.version, expectedDraftVersion: tree.draftVersion });
      const next = await bridge.getContextTree({ workspaceId: mission.workspaceId });
      if (alive.current && request === contextRead.current) setTree(next);
    } catch (reason) {
      if (alive.current) { setError(message(reason)); setReload((value) => value + 1); }
    } finally {
      inFlight.current = false;
      if (alive.current) setWorking(false);
    }
  };

  const buildPreview = async () => {
    if (inFlight.current || busy || !valid) return;
    invalidate();
    const expectedRevision = revision.current;
    inFlight.current = true;
    setWorking(true);
    try {
      const result = await bridge.previewAgentHandoff({ missionId: mission.id, title: title.trim(), spec: spec.trim(), context: { includeActiveContext, evidenceIds } });
      if (alive.current && expectedRevision === revision.current) setPreview(result);
    } catch (reason) {
      if (alive.current && expectedRevision === revision.current) setError(message(reason));
    } finally {
      inFlight.current = false;
      if (alive.current) setWorking(false);
    }
  };

  const start = async () => {
    if (inFlight.current || busy || !canStart || !valid || !preview) return;
    inFlight.current = true;
    setWorking(true);
    setError(null);
    const confirmed = preview;
    // The confirmation is consumed once; another dispatch always needs a fresh preview.
    setPreview(null);
    revision.current += 1;
    try {
      const operation = await onStart({ missionId: mission.id, title: title.trim(), spec: spec.trim(), handoff: {
        context: { includeActiveContext, evidenceIds }, previewHash: confirmed.hash,
        expectedCursorVersion: confirmed.cursorVersion, expectedDraftVersion: confirmed.draftVersion,
        expectedBranchId: confirmed.branchId, expectedBranchVersion: confirmed.branchVersion,
      } });
      if (!alive.current) return;
      if (operation?.status === "succeeded") { setTitle(""); setSpec(""); setEvidenceIds([]); setIncludeActiveContext(false); }
      setReload((value) => value + 1);
    } catch (reason) {
      if (alive.current) { setError(`${message(reason)} 请重新预览后再启动。`); setReload((value) => value + 1); }
    } finally {
      inFlight.current = false;
      if (alive.current) setWorking(false);
    }
  };

  return <>
    <form className="agent-workspace__panel" onSubmit={(event) => { event.preventDefault(); void start(); }}>
      <h4>预览上下文，交给 Agent</h4>
      <p>沿用本机 OMP 的模型、认证、工具、Skills 与审批设置。工具批准在 Orca / Agent 中处理；实际使用的模型由 Agent 决定。</p>
      <label>任务标题<input aria-label="任务标题" aria-invalid={!!titleError} required value={title} onChange={(event) => { setTitle(event.target.value); invalidate(); }} disabled={busy} /></label>
      {titleError ? <p className="agent-workspace__error" role="alert">{titleError}</p> : null}
      <label>任务说明<textarea aria-label="任务说明" aria-invalid={!!specError} required rows={4} value={spec} onChange={(event) => { setSpec(event.target.value); invalidate(); }} placeholder="写清范围、约束和验收标准" disabled={busy} /></label>
      {specError ? <p className="agent-workspace__error" role="alert">{specError}</p> : null}
      <fieldset disabled={busy} className="agent-workspace__selection">
        <legend>选择本次交接内容</legend>
        <label><input type="checkbox" checked={includeActiveContext} onChange={(event) => { setIncludeActiveContext(event.target.checked); invalidate(); }} />包含当前图谱路径</label>
        <p>只发送明确勾选的内容。切换路径后请重新预览；新工作区可以只发送任务说明。</p>
        {tree ? <ContextTree projection={tree} busy={busy || working} onSelect={selectContext} /> : <p>尚未读取图谱路径，可重新加载。</p>}
        <h4>选择已保存的 Agent 证据</h4>
        {!evidence.length ? <p>尚无保存的输出观察；任务执行后可以保存一页，作为下一任务输入。</p> : null}
        {evidence.map((item) => <label key={item.id}>
          <input type="checkbox" aria-label={`使用证据 ${evidenceTitle(item)}`} data-evidence-id={item.id} checked={evidenceIds.includes(item.id)} onChange={(event) => { setEvidenceIds((ids) => event.target.checked ? [...ids, item.id] : ids.filter((id) => id !== item.id)); invalidate(); }} />
          {evidenceTitle(item)} · {item.createdAt} · {item.hasMore ? "部分输出，有后续页" : "一页输出观察"}
        </label>)}
      </fieldset>
      {error ? <p className="agent-workspace__error" role="alert">{error}</p> : null}
      <div className="agent-workspace__actions">
        <button type="button" disabled={busy || working || !valid} onClick={() => void buildPreview()}>{working && !busy ? "正在处理…" : "预览交接内容"}</button>
        <button className="agent-workspace__primary" disabled={busy || working || !canStart || !valid || !preview} type="submit">{busy ? "正在提交…" : "启动 OMP 任务"}</button>
      </div>
      {preview ? <section aria-label="交接预览"><h4>即将提交给 Agent 的文本</h4><pre>{preview.text}</pre><p>来源回答 {preview.sourceRunIds.length} · 输出观察 {preview.evidenceIds.length} · hash <code>{preview.hash}</code></p><p>这是提交给 harness 的交接快照；Agent 的后续工具调用和完整模型请求由现有运行时管理。</p></section> : <p>启动前必须先查看交接预览；编辑任务或所选来源会使预览失效。</p>}
      <p>启动后，OMP 可在独立 worktree 中读写文件；不会自动合并，也不会使用 ThoughtsFlow 的 Provider 凭据。</p>
    </form>
    {historyError ? <p className="agent-workspace__error" role="alert">读取本地上下文记录失败：{historyError}</p> : null}
    <button type="button" disabled={busy || working} onClick={() => { invalidate(); setReload((value) => value + 1); }}>重新加载上下文与证据</button>
    <AgentWorkflowGraph handoffs={handoffs} evidence={evidence} runTitles={runTitles} />
    <section className="agent-workspace__panel" aria-label="已保存的 Agent 证据">
      <h4>不可变输出观察 · 可离线读取</h4>
      <p>每条记录只证明某次读取所得的一页；即使没有后续页，也不证明任务完整成功。</p>
      {evidence.map((item) => <details key={item.id}><summary>{evidenceTitle(item)} · {item.createdAt}</summary>
        <p>证据 {item.id} · 任务 {item.taskId} · 协作 {item.missionId} · Run {item.runId} · Dispatch {item.dispatchId}</p><p>读取时状态 {item.observedTaskStatus} · 来源 {item.source ?? "未知"} · {item.hasMore ? "还有后续页" : "未报告后续页"}</p>
        <p>请求游标 {item.requestCursor ?? "首页"} · 返回游标 {item.cursor ?? "无"}</p>
        {item.warning ? <p className="agent-workspace__notice">{item.warning}</p> : null}<pre>{item.text}</pre><code>{item.contentHash}</code>
      </details>)}
    </section>
    <section className="agent-workspace__panel" aria-label="历史任务交接">
      <h4>不可变交接历史</h4>
      <p>保留启动尝试的交接内容；是否实际执行请结合操作回执与任务状态判断。</p>
      {handoffs.map((item) => <details key={item.operationId}><summary>{item.createdAt} · {item.preview.title?.trim() || item.operationId.slice(0, 12)}</summary><p>操作 {item.operationId}</p><pre>{item.preview.text}</pre><p>来源回答：{item.preview.sourceRunIds.join(", ") || "无"} · 输出观察：{item.preview.evidenceIds.join(", ") || "无"}</p><code>{item.preview.hash}</code></details>)}
    </section>
  </>;
}
