import {
  FileCheck2,
  ListChecks,
  ShieldCheck,
  Sparkles,
  X,
} from "lucide-react";
import { useMemo, useState } from "react";
import type { FormEvent } from "react";
import type {
  ContextCheckpointKind,
  ContextTreeRunNode,
  ProviderProfile,
} from "../../shared/contracts";
import "./context-tree.css";

export type ManualCheckpointProposal = {
  kind: ContextCheckpointKind;
  sourceRunIds: string[];
  firstKeptRunId: string | null;
  summary: string;
};

export type ProviderCheckpointProposal = {
  sourceRunIds: string[];
  firstKeptRunId: string | null;
  providerProfileId: string;
  summaryPrompt: string;
};

type Props = {
  nodes: ContextTreeRunNode[];
  profiles: ProviderProfile[];
  estimatedTokens: number;
  busy?: boolean;
  onCancel: () => void | Promise<void>;
  onCreateManual: (proposal: ManualCheckpointProposal) => void | Promise<void>;
  onSummarize: (proposal: ProviderCheckpointProposal) => void | Promise<void>;
};

export function ContextMaintenancePanel({
  nodes,
  profiles,
  estimatedTokens,
  busy = false,
  onCancel,
  onCreateManual,
  onSummarize,
}: Props) {
  const pathNodes = useMemo(() => {
    const byId = new Map(nodes.map((node) => [node.runId, node]));
    const active = nodes.find((node) => node.isActive);
    if (!active) return nodes.filter((node) => node.isOnActivePath);
    const path: ContextTreeRunNode[] = [];
    const visited = new Set<string>();
    let cursor: ContextTreeRunNode | undefined = active;
    while (cursor && !visited.has(cursor.runId)) {
      visited.add(cursor.runId);
      path.unshift(cursor);
      cursor = cursor.parentRunId ? byId.get(cursor.parentRunId) : undefined;
    }
    return path;
  }, [nodes]);
  const defaultFirstKept = pathNodes.at(-1)?.runId ?? "";
  const [sourceRunIds, setSourceRunIds] = useState<string[]>(
    pathNodes.slice(0, -1).map((node) => node.runId),
  );
  const [firstKeptRunId, setFirstKeptRunId] = useState(defaultFirstKept);
  const [providerProfileId, setProviderProfileId] = useState(
    profiles.find((profile) => profile.isDefault)?.id ?? profiles[0]?.id ?? "",
  );
  const [mode, setMode] = useState<"manual" | "provider">("manual");
  const [kind, setKind] = useState<ContextCheckpointKind>("compaction");
  const [summary, setSummary] = useState("");
  const [summaryPrompt, setSummaryPrompt] = useState(
    "总结已验证事实、关键决定、未决风险和下一步；不得补写来源中不存在的信息。",
  );

  const selectedProvider = profiles.find((profile) => profile.id === providerProfileId);
  const retainedBoundaryIndex =
    kind === "compaction"
      ? pathNodes.findIndex((node) => node.runId === firstKeptRunId)
      : pathNodes.length;

  const toggleSource = (runId: string) => {
    const index = pathNodes.findIndex((node) => node.runId === runId);
    if (index < 0) return;
    const nextCount = sourceRunIds.includes(runId) ? index : index + 1;
    const maximumCount =
      kind === "compaction"
        ? Math.max(pathNodes.length - 1, 0)
        : pathNodes.length;
    const boundedCount = Math.min(nextCount, maximumCount);
    setSourceRunIds(pathNodes.slice(0, boundedCount).map((node) => node.runId));
    if (kind === "compaction") {
      setFirstKeptRunId(pathNodes[boundedCount]?.runId ?? "");
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (sourceRunIds.length === 0) return;
    const boundary = kind === "compaction" ? (firstKeptRunId || null) : null;
    if (kind === "compaction" && !boundary) return;
    if (mode === "manual") {
      const value = summary.trim();
      if (!value) return;
      void onCreateManual({
        kind,
        sourceRunIds,
        firstKeptRunId: boundary,
        summary: value,
      });
      return;
    }
    if (!providerProfileId) return;
    const prompt = summaryPrompt.trim();
    if (!prompt) return;
    void onSummarize({
      sourceRunIds,
      firstKeptRunId: boundary,
      providerProfileId,
      summaryPrompt: prompt,
    });
  };

  return (
    <aside className="context-maintenance" aria-label="Context 压缩预览">
      <header className="context-maintenance__header">
        <div>
          <span>CONTEXT MAINTENANCE</span>
          <h2>确认来源后再压缩</h2>
        </div>
        <button
          aria-label={busy ? "取消正在生成的 Context 摘要" : "关闭 Context 压缩预览"}
          onClick={() => void onCancel()}
          type="button"
        >
          <X aria-hidden="true" size={16} />
        </button>
      </header>

      <form onSubmit={submit}>
        <section className="context-maintenance__summary" aria-label="压缩影响预览">
          <div>
            <ListChecks aria-hidden="true" size={14} />
            <strong>{sourceRunIds.length} 个来源 Run</strong>
          </div>
          <div>
            <FileCheck2 aria-hidden="true" size={14} />
            <strong>
              {kind === "branch-summary"
                ? "分支摘要不使用保留边界"
                : firstKeptRunId
                ? `从 ${firstKeptRunId} 开始保留原文`
                : "尚无可用的保留边界"}
            </strong>
          </div>
          <div>
            <ShieldCheck aria-hidden="true" size={14} />
            <strong>预计预算 {estimatedTokens.toLocaleString()} tokens</strong>
          </div>
          <div>
            <Sparkles aria-hidden="true" size={14} />
            <strong>
              {mode === "manual"
                ? "人工摘要"
                : selectedProvider
                ? `${selectedProvider.name} · ${selectedProvider.model}`
                : "尚未选择摘要 Provider"}
            </strong>
          </div>
          <p>sha256 将由后端对精确来源清单计算</p>
          <p>来源 Run 必须严格位于保留边界之前；边界与来源不会重叠。</p>
        </section>

        <fieldset className="context-maintenance__sources">
          <legend>来源范围</legend>
          {pathNodes.map((node, index) => (
            <label key={node.runId}>
              <input
                checked={sourceRunIds.includes(node.runId)}
                disabled={
                  busy
                  || (
                    kind === "compaction"
                    && index >= Math.max(retainedBoundaryIndex, 0)
                  )
                }
                onChange={() => toggleSource(node.runId)}
                type="checkbox"
              />
              <span>
                <strong>{node.title?.trim() || node.prompt}</strong>
                <small>{node.runId} · {node.model}</small>
              </span>
            </label>
          ))}
        </fieldset>

        {kind === "compaction" ? (
          <label className="context-maintenance__field">
            <span>保留原文边界</span>
            <select
              aria-label="保留原文边界"
              disabled={busy}
              onChange={(event) => {
                const nextBoundary = event.target.value;
                const nextBoundaryIndex = pathNodes.findIndex(
                  (node) => node.runId === nextBoundary,
                );
                setFirstKeptRunId(nextBoundary);
                setSourceRunIds(
                  pathNodes
                    .slice(0, Math.max(nextBoundaryIndex, 0))
                    .map((node) => node.runId),
                );
              }}
              value={firstKeptRunId}
            >
              {pathNodes.map((node) => (
                <option key={node.runId} value={node.runId}>
                  {node.runId} · {node.title?.trim() || node.prompt}
                </option>
              ))}
            </select>
          </label>
        ) : null}

        <label className="context-maintenance__field">
          <span>维护类型</span>
          <select
            aria-label="维护类型"
            disabled={busy || mode === "provider"}
            onChange={(event) => {
              const nextKind = event.target.value as ContextCheckpointKind;
              setKind(nextKind);
              if (nextKind === "branch-summary") {
                setFirstKeptRunId("");
                return;
              }
              const sourceCount = Math.min(
                sourceRunIds.length,
                Math.max(pathNodes.length - 1, 0),
              );
              setSourceRunIds(
                pathNodes.slice(0, sourceCount).map((node) => node.runId),
              );
              setFirstKeptRunId(pathNodes[sourceCount]?.runId ?? "");
            }}
            value={kind}
          >
            <option value="compaction">路径压缩</option>
            <option value="branch-summary">分支摘要</option>
          </select>
        </label>

        {mode === "provider" ? (
          <label className="context-maintenance__field">
            <span>摘要 Provider</span>
            <select
              aria-label="摘要 Provider"
              disabled={busy}
              onChange={(event) => setProviderProfileId(event.target.value)}
              value={providerProfileId}
            >
              {profiles.map((profile) => (
                <option key={profile.id} value={profile.id}>
                  {profile.name} · {profile.model}
                </option>
              ))}
            </select>
          </label>
        ) : null}

        <div className="context-maintenance__mode" role="group" aria-label="摘要方式">
          <button
            aria-pressed={mode === "manual"}
            disabled={busy}
            onClick={() => setMode("manual")}
            type="button"
          >
            人工摘要
          </button>
          <button
            aria-pressed={mode === "provider"}
            disabled={busy}
            onClick={() => {
              const sourceCount = Math.min(
                sourceRunIds.length,
                Math.max(pathNodes.length - 1, 0),
              );
              setKind("compaction");
              setSourceRunIds(
                pathNodes.slice(0, sourceCount).map((node) => node.runId),
              );
              setFirstKeptRunId(pathNodes[sourceCount]?.runId ?? "");
              setMode("provider");
            }}
            type="button"
          >
            Provider 生成摘要
          </button>
        </div>

        {mode === "manual" ? (
          <label className="context-maintenance__field">
            <span>不可变摘要内容</span>
            <textarea
              aria-label="人工摘要"
              disabled={busy}
              onChange={(event) => setSummary(event.target.value)}
              placeholder="只写来源中能够审计的事实、决定与未决事项…"
              rows={5}
              value={summary}
            />
          </label>
        ) : (
          <label className="context-maintenance__field">
            <span>摘要请求</span>
            <textarea
              aria-label="摘要请求"
              disabled={busy}
              onChange={(event) => setSummaryPrompt(event.target.value)}
              rows={5}
              value={summaryPrompt}
            />
          </label>
        )}

        <footer>
          <p>失败、取消或版本冲突不会激活检查点，也不会移动当前 Context。</p>
          <button
            disabled={
              busy
              || sourceRunIds.length === 0
              || (mode === "provider" && !providerProfileId)
              || (kind === "compaction" && !firstKeptRunId)
              || (mode === "manual" ? !summary.trim() : !summaryPrompt.trim())
            }
            type="submit"
          >
            {mode === "manual" ? "保存人工压缩检查点" : "确认生成并切换 Context"}
          </button>
        </footer>
      </form>
    </aside>
  );
}
