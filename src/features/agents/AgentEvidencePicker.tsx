import { memo } from "react";
import { agentHistoryTime } from "./AgentHistory";
import type { AgentEvidenceSummary } from "../../shared/contracts";

export type AgentEvidenceChoice = Pick<AgentEvidenceSummary, "id" | "title" | "taskId" | "createdAt">;

export const MAX_SELECTED_EVIDENCE = 32;

export const AgentEvidencePicker = memo(function AgentEvidencePicker({
  evidence, selected, disabled, loading, pageNumber, hasNext, onPrevious, onNext, onLatest, onToggle,
}: {
  evidence: AgentEvidenceSummary[];
  selected: AgentEvidenceChoice[];
  disabled: boolean;
  loading: boolean;
  pageNumber: number;
  hasNext: boolean;
  onPrevious: () => void;
  onNext: () => void;
  onLatest: () => void;
  onToggle: (item: AgentEvidenceChoice) => void;
}) {
  return <section aria-label="选择已保存的 Agent 证据">
    <h4>选择已保存的 Agent 证据</h4>
    <p>每页最多 50 条摘要；已选 {selected.length} / {MAX_SELECTED_EVIDENCE} 条，跨页及重新加载均保留选择。</p>
    {selected.length ? <div aria-label="已选证据" className="agent-workspace__selected-evidence">
      {selected.map((item) => <button type="button" key={item.id} disabled={disabled} onClick={() => onToggle(item)} aria-label={`取消证据 ${item.title?.trim() || item.taskId.slice(0, 12)}`}>
        {item.title?.trim() || item.taskId.slice(0, 12)} · {agentHistoryTime(item.createdAt)} ×
      </button>)}
    </div> : null}
    {loading ? <p role="status">正在加载证据摘要…</p> : !evidence.length ? <p>这一页没有输出观察；任务执行后可以保存一页，作为下一任务输入。</p> : null}
    {evidence.map((item) => {
      const checked = selected.some((choice) => choice.id === item.id);
      const title = item.title?.trim() || item.taskId.slice(0, 12);
      return <label key={item.id}>
        <input type="checkbox" aria-label={`使用证据 ${title}`} data-evidence-id={item.id} checked={checked} disabled={disabled || (!checked && selected.length >= MAX_SELECTED_EVIDENCE)} onChange={() => onToggle(item)} />
        {title} · {agentHistoryTime(item.createdAt)} · {item.hasMore ? "部分输出，有后续页" : "一页输出观察"}
      </label>;
    })}
    <div className="agent-workspace__actions" aria-label="证据摘要分页">
      <button type="button" disabled={disabled || loading} onClick={onLatest}>查看最新证据</button>
      <button type="button" disabled={disabled || loading || pageNumber === 1} onClick={onPrevious}>上一页证据</button>
      <span>第 {pageNumber} 页</span>
      <button type="button" disabled={disabled || loading || !hasNext} onClick={onNext}>下一页证据</button>
    </div>
  </section>;
});
