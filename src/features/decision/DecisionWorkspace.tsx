import { Check, Download, GitCompareArrows, ShieldQuestion, X } from "lucide-react";
import { useMemo, useState } from "react";
import type {
  CompareRunsResult,
  ContextDiffItem,
  DecisionMark,
  DecisionStatus,
  ExportResult,
} from "../../shared/contracts";
import "./decision-workspace.css";

export type { CompareRunsResult, DecisionStatus } from "../../shared/contracts";

export interface DecisionRunOption {
  runId: string;
  label: string;
  model: string;
  status: string;
}

export type DecisionMarkView = DecisionMark;

export interface MarkDecisionInput {
  workspaceId: string;
  runId: string;
  status: DecisionStatus;
  reason: string;
}

export interface ExportDecisionPacketInput {
  workspaceId: string;
}

export interface DecisionWorkspaceProps {
  workspaceId: string;
  runs: DecisionRunOption[];
  existingMarks: DecisionMarkView[];
  onCompare: (input: {
    leftRunId: string;
    rightRunId: string;
  }) => Promise<CompareRunsResult>;
  onMarkDecision: (input: MarkDecisionInput) => Promise<DecisionMarkView>;
  onExport: (input: ExportDecisionPacketInput) => Promise<ExportResult>;
}

type LineDiff = { kind: "same" | "removed" | "added"; text: string };

const decisionChoices: Array<{
  status: DecisionStatus;
  label: string;
  icon: typeof Check;
}> = [
  { status: "accepted", label: "采纳", icon: Check },
  { status: "rejected", label: "否决", icon: X },
  { status: "to-verify", label: "待验证", icon: ShieldQuestion },
];

function outputLineDiff(left: string, right: string): LineDiff[] {
  const leftLines = left.split("\n");
  const rightLines = right.split("\n");
  if (leftLines.length * rightLines.length > 250_000) {
    return [
      ...leftLines.map((text): LineDiff => ({ kind: "removed", text })),
      ...rightLines.map((text): LineDiff => ({ kind: "added", text })),
    ];
  }
  const rows = leftLines.length + 1;
  const columns = rightLines.length + 1;
  const lengths = Array.from({ length: rows }, () => Array<number>(columns).fill(0));

  for (let leftIndex = leftLines.length - 1; leftIndex >= 0; leftIndex -= 1) {
    for (let rightIndex = rightLines.length - 1; rightIndex >= 0; rightIndex -= 1) {
      lengths[leftIndex][rightIndex] =
        leftLines[leftIndex] === rightLines[rightIndex]
          ? lengths[leftIndex + 1][rightIndex + 1] + 1
          : Math.max(lengths[leftIndex + 1][rightIndex], lengths[leftIndex][rightIndex + 1]);
    }
  }

  const result: LineDiff[] = [];
  let leftIndex = 0;
  let rightIndex = 0;

  while (leftIndex < leftLines.length && rightIndex < rightLines.length) {
    if (leftLines[leftIndex] === rightLines[rightIndex]) {
      result.push({ kind: "same", text: leftLines[leftIndex] });
      leftIndex += 1;
      rightIndex += 1;
    } else if (lengths[leftIndex + 1][rightIndex] >= lengths[leftIndex][rightIndex + 1]) {
      result.push({ kind: "removed", text: leftLines[leftIndex] });
      leftIndex += 1;
    } else {
      result.push({ kind: "added", text: rightLines[rightIndex] });
      rightIndex += 1;
    }
  }

  while (leftIndex < leftLines.length) {
    result.push({ kind: "removed", text: leftLines[leftIndex] });
    leftIndex += 1;
  }
  while (rightIndex < rightLines.length) {
    result.push({ kind: "added", text: rightLines[rightIndex] });
    rightIndex += 1;
  }

  return result;
}

function ContextList({ emptyLabel, items }: { emptyLabel: string; items: ContextDiffItem[] }) {
  if (!items.length) return <p className="decision-empty">{emptyLabel}</p>;

  return (
    <ol className="context-diff-list">
      {items.map((item) => (
        <li key={`${item.id}-${item.ordinal}`}>
          <span className="context-diff-list__ordinal">{item.ordinal}</span>
          <div>
            <strong>{item.source}</strong>
            <span>{item.role}</span>
            <p>{item.preview}</p>
          </div>
        </li>
      ))}
    </ol>
  );
}

interface DecisionEditorProps {
  mark?: DecisionMarkView;
  run: DecisionRunOption;
  workspaceId: string;
  onMarkDecision: (input: MarkDecisionInput) => Promise<DecisionMarkView>;
  onSaved: (mark: DecisionMarkView) => void;
}

function DecisionEditor({
  mark,
  run,
  workspaceId,
  onMarkDecision,
  onSaved,
}: DecisionEditorProps) {
  const [status, setStatus] = useState<DecisionStatus>(mark?.status ?? "to-verify");
  const [reason, setReason] = useState(mark?.reason ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    const cleanReason = reason.trim();
    if (!cleanReason) {
      setError("请写明判断理由，Decision Packet 才能供他人审查。");
      return;
    }

    setSaving(true);
    setError(null);
    try {
      const saved = await onMarkDecision({
        workspaceId,
        runId: run.runId,
        status,
        reason: cleanReason,
      });
      onSaved(saved);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "保存决策标记失败");
    } finally {
      setSaving(false);
    }
  };

  return (
    <article className="decision-editor" aria-label={`${run.label}的决策`}>
      <header>
        <div>
          <h4>{run.label}</h4>
          <code>{run.model}</code>
        </div>
        {mark ? <span className={`decision-editor__saved decision-editor__saved--${mark.status}`}>已保存</span> : null}
      </header>
      <div className="decision-editor__choices" role="group" aria-label={`${run.label}的状态`}>
        {decisionChoices.map((choice) => {
          const Icon = choice.icon;
          return (
            <button
              aria-pressed={status === choice.status}
              className={`decision-choice decision-choice--${choice.status}`}
              key={choice.status}
              onClick={() => setStatus(choice.status)}
              type="button"
            >
              <Icon aria-hidden="true" size={13} />
              {choice.label}
            </button>
          );
        })}
      </div>
      <label>
        判断理由
        <textarea
          aria-label={`${run.label}的判断理由`}
          onChange={(event) => setReason(event.target.value)}
          placeholder="写明证据、取舍或仍需验证的问题"
          rows={3}
          value={reason}
        />
      </label>
      {error ? <p className="decision-error" role="alert">{error}</p> : null}
      <button className="decision-editor__save" disabled={saving} onClick={save} type="button">
        {saving ? "保存中…" : "保存决策标记"}
      </button>
    </article>
  );
}

export function DecisionWorkspace({
  workspaceId,
  runs,
  existingMarks,
  onCompare,
  onMarkDecision,
  onExport,
}: DecisionWorkspaceProps) {
  const [leftRunId, setLeftRunId] = useState(runs[0]?.runId ?? "");
  const [rightRunId, setRightRunId] = useState(runs[1]?.runId ?? "");
  const [comparison, setComparison] = useState<CompareRunsResult | null>(null);
  const [marks, setMarks] = useState(existingMarks);
  const [busy, setBusy] = useState<"compare" | "export" | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const leftRun = runs.find((run) => run.runId === leftRunId);
  const rightRun = runs.find((run) => run.runId === rightRunId);
  const lineDiff = useMemo(
    () =>
      comparison
        ? outputLineDiff(comparison.answer.leftMarkdown, comparison.answer.rightMarkdown)
        : [],
    [comparison],
  );

  const compare = async () => {
    if (!leftRunId || !rightRunId) {
      setNotice("请选择两条路线。");
      return;
    }
    if (leftRunId === rightRunId) {
      setNotice("请选择两个不同的回答版本。");
      return;
    }

    setBusy("compare");
    setNotice(null);
    try {
      setComparison(await onCompare({ leftRunId, rightRunId }));
    } catch (caught) {
      setNotice(caught instanceof Error ? caught.message : "路线比较失败");
    } finally {
      setBusy(null);
    }
  };

  const exportPacket = async () => {
    setBusy("export");
    setNotice(null);
    try {
      const result = await onExport({ workspaceId });
      setNotice(`Decision Packet 已导出：${result.path}`);
    } catch (caught) {
      setNotice(caught instanceof Error ? caught.message : "导出失败");
    } finally {
      setBusy(null);
    }
  };

  const updateMark = (saved: DecisionMarkView) => {
    setMarks((current) => [saved, ...current.filter((mark) => mark.runId !== saved.runId)]);
  };

  return (
    <section className="decision-workspace" aria-label="路线比较与决策">
      <header className="decision-workspace__header">
        <div>
          <span className="decision-workspace__eyebrow">DECISION REVIEW</span>
          <h2>比较路线，形成可审查的判断</h2>
          <p>回答与 Context 分开比较；决策标记不会改写历史 Run。</p>
        </div>
        <button
          className="decision-export"
          disabled={!comparison || busy === "export"}
          onClick={exportPacket}
          type="button"
        >
          <Download aria-hidden="true" size={14} />
          {busy === "export" ? "导出中…" : "导出 Decision Packet"}
        </button>
      </header>

      <div className="decision-picker">
        <label>
          路线 A
          <select aria-label="路线 A" onChange={(event) => setLeftRunId(event.target.value)} value={leftRunId}>
            <option value="">选择回答版本</option>
            {runs.map((run) => (
              <option key={run.runId} value={run.runId}>
                {run.label} · {run.model}
              </option>
            ))}
          </select>
        </label>
        <GitCompareArrows aria-hidden="true" className="decision-picker__icon" size={18} />
        <label>
          路线 B
          <select aria-label="路线 B" onChange={(event) => setRightRunId(event.target.value)} value={rightRunId}>
            <option value="">选择回答版本</option>
            {runs.map((run) => (
              <option key={run.runId} value={run.runId}>
                {run.label} · {run.model}
              </option>
            ))}
          </select>
        </label>
        <button className="decision-compare" disabled={busy === "compare"} onClick={compare} type="button">
          {busy === "compare" ? "比较中…" : "比较两条路线"}
        </button>
      </div>

      {notice ? <p className="decision-notice" role="status">{notice}</p> : null}

      {comparison && leftRun && rightRun ? (
        <>
          <section className="answer-comparison" aria-label="回答差异">
            <article>
              <header>
                <h3>{leftRun.label}</h3>
                <code>{comparison.left.model}</code>
              </header>
              <div className="answer-comparison__markdown">{comparison.answer.leftMarkdown}</div>
            </article>
            <article>
              <header>
                <h3>{rightRun.label}</h3>
                <code>{comparison.right.model}</code>
              </header>
              <div className="answer-comparison__markdown">{comparison.answer.rightMarkdown}</div>
            </article>
          </section>

          <details className="output-diff">
            <summary>查看逐行输出差异</summary>
            <div aria-label="逐行输出差异">
              {lineDiff.map((line, index) => (
                <div className={`output-diff__line output-diff__line--${line.kind}`} key={`${line.kind}-${index}`}>
                  <span>{line.kind === "removed" ? "−" : line.kind === "added" ? "+" : " "}</span>
                  <code>{line.text || " "}</code>
                </div>
              ))}
            </div>
          </details>

          <section className="context-diff" aria-label="Context Diff">
            <header>
              <span className="decision-workspace__eyebrow">CONTEXT DIFF</span>
              <h3>两次运行实际使用的 Context</h3>
            </header>
            <div className="context-diff__columns">
              <article>
                <h4>仅路线 A · {comparison.contextDiff.onlyLeft.length}</h4>
                <ContextList emptyLabel="没有仅路线 A 使用的内容" items={comparison.contextDiff.onlyLeft} />
              </article>
              <article>
                <h4>共同使用 · {comparison.contextDiff.shared.length}</h4>
                <ContextList emptyLabel="没有共同内容" items={comparison.contextDiff.shared} />
              </article>
              <article>
                <h4>仅路线 B · {comparison.contextDiff.onlyRight.length}</h4>
                <ContextList emptyLabel="没有仅路线 B 使用的内容" items={comparison.contextDiff.onlyRight} />
              </article>
            </div>
          </section>

          <section className="decision-marks" aria-label="决策标记">
            <header>
              <span className="decision-workspace__eyebrow">DECISION MARKS</span>
              <h3>说明采纳、否决或待验证的理由</h3>
            </header>
            <div className="decision-marks__grid">
              {[leftRun, rightRun].map((run) => (
                <DecisionEditor
                  key={run.runId}
                  mark={marks.find((mark) => mark.runId === run.runId)}
                  onMarkDecision={onMarkDecision}
                  onSaved={updateMark}
                  run={run}
                  workspaceId={workspaceId}
                />
              ))}
            </div>
          </section>
        </>
      ) : (
        <div className="decision-placeholder">
          <GitCompareArrows aria-hidden="true" size={20} />
          <p>选择两个回答版本，核对它们的结论与实际 Context。</p>
        </div>
      )}
    </section>
  );
}
