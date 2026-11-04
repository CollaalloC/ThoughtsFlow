import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ContextTree } from "../context-tree";
import type { DesktopBridge } from "../../platform/desktop-bridge";
import type { AgentTemplate, AgentHandoffPreview, AgentMission, AgentOperation, ContextTreeProjection, StartAgentTaskInput } from "../../shared/contracts";
import { AgentWorkflowGraph } from "./AgentWorkflowGraph";
import { AgentHistory } from "./AgentHistory";
import { AgentEvidencePicker, MAX_SELECTED_EVIDENCE, type AgentEvidenceChoice } from "./AgentEvidencePicker";
import { useAgentHistoryPage } from "./useAgentHistoryPage";
import type { AgentTaskDraft, AgentTaskDrafts } from "./agent-drafts";

interface Props {
  templates?: AgentTemplate[];
  bridge: DesktopBridge;
  mission: AgentMission;
  busy: boolean;
  canStart: boolean;
  evidenceRevision: number;
  drafts?: AgentTaskDrafts;
  onStart: (input: Omit<StartAgentTaskInput, "operationId">) => Promise<AgentOperation | undefined>;
}

function message(reason: unknown) {
  return reason instanceof Error ? reason.message : "无法读取任务上下文。";
}

function byteLimit(label: string, value: string, limit: number) {
  const bytes = new TextEncoder().encode(value.trim()).length;
  return bytes > limit ? `${label}超过 ${limit} 字节上限（当前 ${bytes} 字节），请缩短后再提交。` : null;
}

function templateValidation(value: string | undefined) {
  return value && new TextEncoder().encode(value).length <= 128 && !value.startsWith(".") && !/[\\/\u0000-\u001f\u007f-\u009f]/.test(value)
    ? null : "请选择已有 herdr-core Agent 模板。";
}

export function AgentHandoffPanel(props: Props) {
  return <AgentHandoffContent key={`${props.mission.workspaceId}:${props.mission.id}`} {...props} />;
}

function AgentHandoffContent({ templates = [], bridge, mission, busy, canStart, evidenceRevision, onStart, drafts }: Props) {
  const initialDraft = useRef<AgentTaskDraft>(drafts?.get(mission.id) ?? { title: "", spec: "", includeActiveContext: false, evidenceIds: [] });
  const draft = useRef(initialDraft.current);
  const observedSavedDraft = useRef(drafts?.get(mission.id));
  const [title, setTitle] = useState(initialDraft.current.title);
  const [spec, setSpec] = useState(initialDraft.current.spec);
  const [requestedTemplate, setRequestedTemplate] = useState(initialDraft.current.requestedTemplate);
  const [includeActiveContext, setIncludeActiveContext] = useState(initialDraft.current.includeActiveContext);
  const [evidenceIds, setEvidenceIds] = useState(initialDraft.current.evidenceIds);
  const selectedLabels = useRef(new Map<string, AgentEvidenceChoice>());
  const persistDraft = useCallback((change: Partial<AgentTaskDraft>) => {
    draft.current = { ...draft.current, ...change };
    drafts?.set(mission.id, draft.current);
  }, [drafts, mission.id]);
  const [tree, setTree] = useState<ContextTreeProjection | null>(null);
  const runTitles = useMemo(() => Object.fromEntries((tree?.nodes ?? []).map((node) => [node.runId, node.title?.trim() || node.prompt.trim().slice(0, 60)])), [tree]);
  const [preview, setPreview] = useState<AgentHandoffPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [contextError, setContextError] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  const [reload, setReload] = useState(0);
  const loadEvidence = useCallback((cursor?: string) => bridge.listAgentEvidencePage({ workspaceId: mission.workspaceId, cursor }), [bridge, mission.workspaceId]);
  const loadHandoffs = useCallback((cursor?: string) => bridge.listAgentHandoffPage({ missionId: mission.id, cursor }), [bridge, mission.id]);
  const evidencePage = useAgentHistoryPage(loadEvidence, `${evidenceRevision}:${reload}`);
  const handoffPage = useAgentHistoryPage(loadHandoffs, `${evidenceRevision}:${reload}`);
  const evidence = evidencePage.items;
  const handoffs = handoffPage.items;
  const selectedEvidence = useMemo(() => evidenceIds.map((id) => evidence.find((item) => item.id === id)
    ?? selectedLabels.current.get(id) ?? { id, taskId: id, title: null, createdAt: "已选，来源摘要尚未加载" }), [evidenceIds, evidence]);
  useEffect(() => {
    evidence.filter((item) => evidenceIds.includes(item.id)).forEach((item) => selectedLabels.current.set(item.id, item));
  }, [evidence, evidenceIds]);
  const alive = useRef(true);
  const revision = useRef(0);
  const contextRead = useRef(0);
  const inFlight = useRef(false);
  const titleError = byteLimit("任务标题", title, 200);
  const specError = byteLimit("任务说明", spec, 64_000);
  const templateError = templateValidation(requestedTemplate);
  const valid = !!title.trim() && !!spec.trim() && !titleError && !specError && !templateError;
  const invalidate = useCallback(() => { revision.current += 1; setPreview(null); setError(null); }, []);

  useEffect(() => {
    if (!drafts) return;
    const reconcileDeletion = () => {
      const saved = drafts.get(mission.id);
      if (observedSavedDraft.current && !saved) {
        draft.current = { title: "", spec: "", includeActiveContext: false, evidenceIds: [] };
        setTitle(""); setSpec(""); setRequestedTemplate(undefined); setIncludeActiveContext(false); setEvidenceIds([]);
        selectedLabels.current.clear();
        invalidate();
      }
      observedSavedDraft.current = saved;
    };
    const unsubscribe = drafts.subscribe(reconcileDeletion);
    reconcileDeletion();
    return unsubscribe;
  }, [drafts, mission.id, invalidate]);

  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; revision.current += 1; };
  }, []);

  useEffect(() => {
    let active = true;
    const request = ++contextRead.current;
    void bridge.getContextTree({ workspaceId: mission.workspaceId }).then((context) => {
      if (!active || request !== contextRead.current) return;
      setTree(context);
      setContextError(null);
    }).catch((reason: unknown) => {
      if (active && request === contextRead.current) setContextError(message(reason));
    });
    return () => { active = false; };
  }, [bridge, mission.workspaceId, reload]);

  const toggleEvidence = useCallback((item: AgentEvidenceChoice) => {
    const ids = draft.current.evidenceIds;
    const selected = ids.includes(item.id);
    if (!selected && ids.length >= MAX_SELECTED_EVIDENCE) return;
    const next = selected ? ids.filter((id) => id !== item.id) : [...ids, item.id];
    if (selected) selectedLabels.current.delete(item.id); else selectedLabels.current.set(item.id, item);
    setEvidenceIds(next);
    persistDraft({ evidenceIds: next });
    invalidate();
  }, [persistDraft, invalidate]);

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
      const result = await bridge.previewAgentHandoff({ missionId: mission.id, title: title.trim(), spec: spec.trim(),
        ...(requestedTemplate === undefined ? {} : { requestedTemplate }), context: { includeActiveContext, evidenceIds } });
      if (alive.current && expectedRevision === revision.current) {
        if (requestedTemplate !== undefined && result.requestedTemplate !== requestedTemplate) throw new Error("交接预览未确认所选模板，请重新预览。");
        setPreview(result);
      }
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
      const operation = await onStart({ missionId: mission.id, title: title.trim(), spec: spec.trim(),
        ...(requestedTemplate === undefined ? {} : { requestedTemplate }), handoff: {
        context: { includeActiveContext, evidenceIds }, previewHash: confirmed.hash,
        expectedCursorVersion: confirmed.cursorVersion, expectedDraftVersion: confirmed.draftVersion,
        expectedBranchId: confirmed.branchId, expectedBranchVersion: confirmed.branchVersion,
      } });
      if (!alive.current) return;
      if (operation?.status === "succeeded") {
        setTitle(""); setSpec(""); setRequestedTemplate(undefined); setEvidenceIds([]); setIncludeActiveContext(false);
        draft.current = { title: "", spec: "", evidenceIds: [], includeActiveContext: false };
        selectedLabels.current.clear();
        drafts?.delete(mission.id);
      }
      setReload((value) => value + 1);
    } catch (reason) {
      if (alive.current) { setError(`${message(reason)} 请重新预览后再启动。`); setReload((value) => value + 1); }
    } finally {
      inFlight.current = false;
      if (alive.current) setWorking(false);
    }
  };

  return <>
    {mission.runtimeBackend === "herdr-core" ? <form className="agent-workspace__panel" onSubmit={(event) => { event.preventDefault(); void start(); }}>
      <h4>预览上下文，交给 Agent</h4>
      <p>选择现有 Herdr 模板，继承其 Agent、模型、认证、工具和 Skills 配置。</p>
      <label>任务标题<input aria-label="任务标题" aria-invalid={!!titleError} required value={title} onChange={(event) => { setTitle(event.target.value); persistDraft({ title: event.target.value }); invalidate(); }} disabled={busy} /></label>
      {titleError ? <p className="agent-workspace__error" role="alert">{titleError}</p> : null}
      <label>任务说明<textarea aria-label="任务说明" aria-invalid={!!specError} required rows={4} value={spec} onChange={(event) => { setSpec(event.target.value); persistDraft({ spec: event.target.value }); invalidate(); }} placeholder="写清范围、约束和验收标准" disabled={busy} /></label>
      {specError ? <p className="agent-workspace__error" role="alert">{specError}</p> : null}
      <label>Agent 模板<select aria-label="Agent 模板" value={requestedTemplate ?? ""} required disabled={busy} onChange={(event) => {
        setRequestedTemplate(event.target.value); persistDraft({ requestedTemplate: event.target.value }); invalidate();
      }}><option value="">选择已有模板</option>
        {requestedTemplate && !templates.some((item) => item.name === requestedTemplate) ? <option value={requestedTemplate}>{requestedTemplate}（尚未核实）</option> : null}
        {templates.map((item) => <option key={item.name} value={item.name}>{item.name} · {item.description}</option>)}
      </select></label>
      {!templates.length ? <p>连接 herdr-core 后读取本机模板；模型和 Agent 启动选项在 Herdr 模板中配置。</p> : null}
      <fieldset disabled={busy} className="agent-workspace__selection">
        <legend>选择本次交接内容</legend>
        <label><input type="checkbox" checked={includeActiveContext} onChange={(event) => { setIncludeActiveContext(event.target.checked); persistDraft({ includeActiveContext: event.target.checked }); invalidate(); }} />包含当前图谱路径</label>
        <p>只发送明确勾选的内容。切换路径后请重新预览；新工作区可以只发送任务说明。</p>
        {includeActiveContext ? tree?.nodes.length ? <ContextTree projection={tree} busy={busy || working} onSelect={selectContext} />
          : <p>{tree ? "当前工作区还没有对话路径，将只交接任务说明和已选证据。" : "正在读取图谱路径…"}</p> : null}
        <AgentEvidencePicker evidence={evidence} selected={selectedEvidence} disabled={busy}
          loading={evidencePage.loading} pageNumber={evidencePage.pageNumber} hasNext={!!evidencePage.nextCursor}
          onPrevious={evidencePage.previous} onNext={evidencePage.next} onLatest={evidencePage.latest} onToggle={toggleEvidence} />
      </fieldset>
      {error ? <p className="agent-workspace__error" role="alert">{error}</p> : null}
      <div className="agent-workspace__actions">
        <button type="button" disabled={busy || working || !valid} onClick={() => void buildPreview()}>{working && !busy ? "正在处理…" : "预览交接内容"}</button>
        <button className="agent-workspace__primary" disabled={busy || working || !canStart || !valid || !preview} type="submit">{busy ? "正在提交…" : "启动 Agent 任务"}</button>
      </div>
      {preview ? <section aria-label="交接预览"><h4>即将提交给 Agent 的文本</h4><p>请求模板：{preview.requestedTemplate ?? "历史交接"}（模型由模板中的 Agent 决定）</p><pre>{preview.text}</pre><p>来源回答 {preview.sourceRunIds.length} · 输出观察 {preview.evidenceIds.length} · hash <code>{preview.hash}</code></p><p>这是提交给 harness 的交接快照；Agent 的后续工具调用和完整模型请求由现有运行时管理。</p></section> : <p>启动前必须先查看交接预览；编辑任务、模板或所选来源会使预览失效。</p>}
      <p>Agent 在协作选定的目录中运行，该目录可能与其他任务共享；不会自动创建 worktree。需要隔离时请选择独立 checkout。</p>
    </form> : <p>旧 Orca 协作仅供查阅，原始交接和证据保留；新任务请建立 herdr-core 协作。</p>}
    {[contextError, evidencePage.error, handoffPage.error].filter(Boolean).map((item, index) => <p key={index} className="agent-workspace__error" role="alert">读取本地上下文记录失败：{item}</p>)}
    <button type="button" disabled={busy || working} onClick={() => { invalidate(); setReload((value) => value + 1); }}>重新加载上下文与证据</button>
    <AgentWorkflowGraph handoffs={handoffs} evidence={evidence} runTitles={runTitles} />
    <AgentHistory bridge={bridge} workspaceId={mission.workspaceId} missionId={mission.id} evidence={evidence} handoffs={handoffs} />
    <div className="agent-workspace__actions" aria-label="交接摘要分页">
      <button type="button" disabled={busy || handoffPage.loading} onClick={handoffPage.latest}>查看最新交接</button>
      <button type="button" disabled={busy || handoffPage.loading || handoffPage.pageNumber === 1} onClick={handoffPage.previous}>上一页交接</button>
      <span>第 {handoffPage.pageNumber} 页 · 每页最多 50 条</span>
      <button type="button" disabled={busy || handoffPage.loading || !handoffPage.nextCursor} onClick={handoffPage.next}>下一页交接</button>
    </div>
  </>;
}
