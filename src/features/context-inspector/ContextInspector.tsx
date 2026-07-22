import {
  Check,
  Clock3,
  Eye,
  EyeOff,
  FileCheck2,
  Pin,
  ShieldAlert,
  X,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { ProviderDestination } from "../../shared/ui";
import "./context-inspector.css";

export type ContextInspectorItem = {
  key: string;
  ordinal: number;
  label: string;
  source: string;
  role: string;
  content: string;
  reason: string;
  estimatedTokens: number;
  included: boolean;
  pinned: boolean;
};

export type LockedSnapshot = {
  canonicalHash: string;
  createdAt: number | string;
  providerName: string;
  model: string;
  baseUrl: string;
  items: ContextInspectorItem[];
};

export type InspectorRun = {
  id: string;
  label: string;
  status: string;
  model: string;
  createdAt: number | string;
  error?: string;
};

export type InspectorTab = "next" | "snapshot" | "runs";

type Props = {
  open: boolean;
  items: ContextInspectorItem[];
  estimatedTokens: number;
  limitTokens: number;
  warnings: string[];
  blocked: boolean;
  provider: { name: string; model: string; baseUrl: string; local: boolean };
  snapshot: LockedSnapshot | null;
  snapshotLoading?: boolean;
  runs: InspectorRun[];
  onClose: () => void;
  onTabChange?: (tab: InspectorTab) => void;
  onToggleIncluded: (item: ContextInspectorItem) => void;
  onTogglePinned: (item: ContextInspectorItem) => void;
};

const tabs: Array<{ id: InspectorTab; label: string }> = [
  { id: "next", label: "下次发送" },
  { id: "snapshot", label: "本次实际发送的内容" },
  { id: "runs", label: "运行记录" },
];

function dateTime(value: number | string) {
  const date = new Date(typeof value === "number" && value < 1e12 ? value * 1000 : value);
  return Number.isNaN(date.getTime())
    ? "未知时间"
    : new Intl.DateTimeFormat("zh-CN", {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      }).format(date);
}

export function ContextInspector({
  open,
  items,
  estimatedTokens,
  limitTokens,
  warnings,
  blocked,
  provider,
  snapshot,
  snapshotLoading = false,
  runs,
  onClose,
  onTabChange,
  onToggleIncluded,
  onTogglePinned,
}: Props) {
  const [tab, setTab] = useState<InspectorTab>("next");
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);

  useEffect(() => {
    if (!open) setTab("next");
  }, [open]);

  if (!open) return null;

  const includedCount = items.filter((item) => item.included).length;
  const pinnedCount = items.filter((item) => item.pinned).length;
  const excludedCount = items.length - includedCount;
  const shownItems = tab === "snapshot" ? snapshot?.items ?? [] : items;
  const snapshotProvider = snapshot
    ? {
        name: snapshot.providerName,
        model: snapshot.model,
        baseUrl: snapshot.baseUrl,
        local: (() => {
          try {
            return ["localhost", "127.0.0.1", "[::1]", "::1"].includes(new URL(snapshot.baseUrl).hostname);
          } catch {
            return false;
          }
        })(),
      }
    : null;
  const displayedProvider = tab === "snapshot" && snapshotProvider ? snapshotProvider : provider;

  const selectTab = (nextTab: InspectorTab) => {
    setTab(nextTab);
    onTabChange?.(nextTab);
  };

  const moveTab = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const delta = event.key === "ArrowRight" ? 1 : -1;
    const nextIndex = (index + delta + tabs.length) % tabs.length;
    selectTab(tabs[nextIndex].id);
    tabRefs.current[nextIndex]?.focus();
  };

  return (
    <aside className="context-inspector" aria-label="Context Inspector">
      <header className="context-inspector__header">
        <div>
          <span>发送前凭证</span>
          <h2>Context Inspector</h2>
        </div>
        <button type="button" className="context-inspector__icon" onClick={onClose} aria-label="关闭上下文检查器">
          <X size={17} />
        </button>
      </header>

      <section className={`context-inspector__meter ${blocked ? "is-blocked" : ""}`} aria-label="上下文用量">
        <div>
          <span>{includedCount} 项将发送</span>
          <strong>约 {estimatedTokens.toLocaleString()} tokens</strong>
        </div>
        <div className="context-inspector__meter-track" aria-hidden="true">
          <span style={{ width: `${Math.min(100, (estimatedTokens / Math.max(1, limitTokens)) * 100)}%` }} />
        </div>
        <small>
          <span><Pin size={11} /> {pinnedCount} 固定</span>
          <span><EyeOff size={11} /> {excludedCount} 排除</span>
          <span>{limitTokens.toLocaleString()} 上限</span>
        </small>
      </section>

      {warnings.length > 0 && (
        <div className="context-inspector__warnings" role={blocked ? "alert" : "status"}>
          <ShieldAlert size={15} aria-hidden="true" />
          <div>{warnings.map((warning) => <p key={warning}>{warning}</p>)}</div>
        </div>
      )}

      <div className="context-inspector__tabs" role="tablist" aria-label="上下文视图">
        {tabs.map((item, index) => (
          <button
            key={item.id}
            ref={(node) => { tabRefs.current[index] = node; }}
            type="button"
            role="tab"
            id={`context-tab-${item.id}`}
            aria-controls={`context-panel-${item.id}`}
            aria-selected={tab === item.id}
            tabIndex={tab === item.id ? 0 : -1}
            className={tab === item.id ? "is-active" : ""}
            onClick={() => selectTab(item.id)}
            onKeyDown={(event) => moveTab(event, index)}
          >
            {item.label}
          </button>
        ))}
      </div>

      <div
        className="context-inspector__body"
        role="tabpanel"
        id={`context-panel-${tab}`}
        aria-labelledby={`context-tab-${tab}`}
      >
        {tab === "snapshot" && (
          <div className="context-inspector__snapshot-heading">
            {snapshotLoading ? (
              <span>正在读取不可变凭证…</span>
            ) : snapshot ? (
              <>
                <span><FileCheck2 size={14} /> {dateTime(snapshot.createdAt)} 锁定</span>
                <code>{snapshot.canonicalHash}</code>
                <ProviderDestination {...snapshotProvider!} />
              </>
            ) : (
              <span>该回答还没有可用快照</span>
            )}
          </div>
        )}

        {tab !== "runs" && (
          <div className="context-inspector__items">
            {shownItems.map((item) => (
              <article
                key={`${tab}-${item.key}`}
                className={`context-item ${item.included ? "is-included" : "is-excluded"} ${item.pinned ? "is-pinned" : ""}`}
              >
                <span className="context-item__ordinal">{item.ordinal}</span>
                <div className="context-item__copy">
                  <div className="context-item__heading">
                    <strong>{item.label}</strong>
                    <span>{item.estimatedTokens} tok</span>
                  </div>
                  <span className="context-item__source">{item.source}</span>
                  <p>{item.content}</p>
                  <span className="context-item__reason">{item.reason}</span>
                </div>
                <div className="context-item__actions">
                  <button
                    type="button"
                    aria-label={`${item.pinned ? "取消固定" : "固定"} ${item.label}`}
                    aria-pressed={item.pinned}
                    disabled={tab === "snapshot"}
                    className={item.pinned ? "is-active is-pin" : ""}
                    onClick={() => onTogglePinned(item)}
                  >
                    <Pin size={13} />
                  </button>
                  <button
                    type="button"
                    aria-label={`${item.included ? "排除" : "重新纳入"} ${item.label}`}
                    aria-pressed={item.included}
                    disabled={tab === "snapshot"}
                    className={item.included ? "is-active" : ""}
                    onClick={() => onToggleIncluded(item)}
                  >
                    {item.included ? <Eye size={13} /> : <EyeOff size={13} />}
                  </button>
                </div>
              </article>
            ))}
          </div>
        )}

        {tab === "runs" && (
          <ol className="context-inspector__runs">
            {runs.map((run) => (
              <li key={run.id}>
                <span className={`context-inspector__run-status is-${run.status}`} aria-hidden="true" />
                <div>
                  <strong>{run.label}</strong>
                  <span>{run.model} · {dateTime(run.createdAt)}</span>
                  {run.error && <p>{run.error}</p>}
                </div>
                {run.status === "completed" ? <Check size={14} /> : <Clock3 size={14} />}
              </li>
            ))}
            {runs.length === 0 && <li className="context-inspector__empty">当前路线还没有运行记录</li>}
          </ol>
        )}
      </div>

      <footer className="context-inspector__footer">
        <div>
          <span>本轮 Context 发送至</span>
          <ProviderDestination {...displayedProvider} />
        </div>
        <p>工作区内容保存在本机；只有上方纳入的 Context 会随本轮请求发往该端点。</p>
      </footer>
    </aside>
  );
}
