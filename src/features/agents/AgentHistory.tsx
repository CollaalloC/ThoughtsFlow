import { memo, useEffect, useRef, useState, type ReactNode } from "react";
import type { DesktopBridge } from "../../platform/desktop-bridge";
import type { AgentEvidenceSummary, AgentHandoffSummary } from "../../shared/contracts";

interface Props {
  bridge: DesktopBridge;
  workspaceId: string;
  missionId: string;
  evidence: AgentEvidenceSummary[];
  handoffs: AgentHandoffSummary[];
}

export function agentHistoryTime(value: string) {
  return Number.isNaN(Date.parse(value)) ? value : new Date(value).toLocaleString();
}

function HistoryEntry<T>({ summary, load, children }: {
  summary: ReactNode;
  load: () => Promise<T>;
  children: (detail: T) => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const [detail, setDetail] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const active = useRef(true);
  const inFlight = useRef(false);

  useEffect(() => {
    active.current = true;
    return () => { active.current = false; };
  }, []);

  const read = async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setLoading(true);
    setError(null);
    try {
      const result = await load();
      if (active.current) setDetail(result);
    } catch (reason) {
      if (active.current) setError(reason instanceof Error && reason.message ? reason.message : "无法读取历史详情。");
    } finally {
      inFlight.current = false;
      if (active.current) setLoading(false);
    }
  };

  return <details onToggle={(event) => {
    const expanded = event.currentTarget.open;
    setOpen(expanded);
    if (expanded && detail === null && error === null) void read();
  }}>
    <summary>{summary}</summary>
    {open ? <>
      {loading ? <p role="status">正在读取历史详情…</p> : null}
      {error ? <div><p className="agent-workspace__error" role="alert">{error}</p><button type="button" onClick={() => void read()} disabled={loading}>重试读取</button></div> : null}
      {detail !== null ? children(detail) : null}
    </> : null}
  </details>;
}

export const AgentHistory = memo(function AgentHistory({ bridge, workspaceId, missionId, evidence, handoffs }: Props) {
  return <>
    <section className="agent-workspace__panel" aria-label="已保存的 Agent 证据">
      <h4>不可变输出观察 · 可离线读取</h4>
      <p>仅展示当前页记录，展开后读取正文。每条记录只证明某次读取所得的一页；即使没有后续页，也不证明任务完整成功。</p>
      {evidence.filter((item) => item.workspaceId === workspaceId).map((item) => <HistoryEntry
        key={`${workspaceId}:${missionId}:${item.id}`}
        summary={<>{item.title?.trim() || item.taskId.slice(0, 12)} · {agentHistoryTime(item.createdAt)}</>}
        load={async () => {
          const result = await bridge.getAgentEvidence({ workspaceId, id: item.id });
          if (result.id !== item.id || result.workspaceId !== workspaceId || result.missionId !== item.missionId) throw new Error("证据详情与所选记录不一致，请重新加载历史记录。");
          return result;
        }}
      >{(detail) => <>
        <p>记录时间 {detail.createdAt}</p>
        <p>证据 {detail.id} · 任务 {detail.taskId} · 协作 {detail.missionId} · Run {detail.runId} · Dispatch {detail.dispatchId}</p>
        <p>交接操作 {detail.handoffOperationId ?? "无"} · 工作区 {detail.workspaceId}</p>
        <p>读取时状态 {detail.observedTaskStatus} · 来源 {detail.source ?? "未知"} · {detail.hasMore ? "还有后续页" : "未报告后续页"}</p>
        <p>请求游标 {detail.requestCursor ?? "首页"} · 返回游标 {detail.cursor ?? "无"}</p>
        {detail.warning ? <p className="agent-workspace__notice">{detail.warning}</p> : null}
        <pre>{detail.text}</pre><code>{detail.contentHash}</code>
      </>}</HistoryEntry>)}
    </section>
    <section className="agent-workspace__panel" aria-label="历史任务交接">
      <h4>不可变交接历史</h4>
      <p>仅展示当前页记录，展开后读取正文。保留启动尝试的交接内容；是否实际执行请结合操作回执与任务状态判断。</p>
      {handoffs.filter((item) => item.workspaceId === workspaceId && item.missionId === missionId).map((item) => <HistoryEntry
        key={`${workspaceId}:${missionId}:${item.operationId}`}
        summary={<>{item.title?.trim() || item.operationId.slice(0, 12)} · {agentHistoryTime(item.createdAt)}</>}
        load={async () => {
          const result = await bridge.getAgentHandoff({ missionId, operationId: item.operationId });
          if (result.operationId !== item.operationId || result.missionId !== missionId || result.workspaceId !== workspaceId) throw new Error("交接详情与所选记录不一致，请重新加载历史记录。");
          return result;
        }}
      >{(detail) => <>
        <p>记录时间 {detail.createdAt}</p>
        <p>操作 {detail.operationId} · 协作 {detail.missionId} · 工作区 {detail.workspaceId}</p>
        {detail.preview.requestedTemplate ? <p>请求模板：{detail.preview.requestedTemplate} · 配置由 Agent 管理</p> : null}
        <p>请求模型：{detail.preview.requestedModel ?? "沿用 Agent 配置"}（不代表实际使用的模型）</p>
        <pre>{detail.preview.text}</pre>
        <p>来源回答：{detail.preview.sourceRunIds.join(", ") || "无"} · 输出观察：{detail.preview.evidenceIds.join(", ") || "无"}</p>
        <code>{detail.preview.hash}</code>
      </>}</HistoryEntry>)}
    </section>
  </>;
});
