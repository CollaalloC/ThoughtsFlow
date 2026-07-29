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
import type {
  ContextCheckpointView,
  ContextItem,
  ProviderAuthPlacement,
  ProviderStreamProtocol,
} from "../../shared/contracts";
import { ProviderDestination } from "../../shared/ui";
import "./context-inspector.css";

export type ContextInspectorItem = ContextItem;

export type LockedSnapshotProviderMetadata =
  | {
      providerMetadataStatus: "resolved";
      providerId: string;
      templateRevision: number;
      streamProtocol: ProviderStreamProtocol;
      authPlacement: ProviderAuthPlacement;
      authHeaderName: string | null;
      additionalHeaders: Record<string, string>;
      parameters: Record<string, unknown>;
    }
  | {
      providerMetadataStatus: "legacy";
      providerId: "legacy";
      templateRevision: 0;
      streamProtocol: "unknown";
      authPlacement: "unknown";
      authHeaderName: null;
      additionalHeaders: Record<string, never>;
      parameters: Record<string, unknown>;
    };

export type LockedSnapshot = {
  canonicalHash: string;
  createdAt: number | string;
  providerName: string;
  model: string;
  baseUrl: string;
  items: ContextInspectorItem[];
} & LockedSnapshotProviderMetadata;

export type InspectorRun = {
  id: string;
  label: string;
  status: string;
  model: string;
  createdAt: number | string;
  error?: string;
};

export type InspectorTab = "raw" | "next" | "snapshot" | "runs";

type Props = {
  open: boolean;
  items: ContextInspectorItem[];
  rawItems?: ContextInspectorItem[];
  draftVersion?: number;
  appliedCheckpoint?: ContextCheckpointView | null;
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
  { id: "raw", label: "原始路径" },
  { id: "next", label: "下一轮实际上下文" },
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

const protocolLabels: Record<ProviderStreamProtocol, string> = {
  openai_sse: "OpenAI SSE",
  ollama_ndjson: "Ollama NDJSON",
  anthropic_sse: "Anthropic SSE",
  google_sse: "Google SSE",
};

const sensitiveHeaderPattern = /^(authorization|proxy-authorization|cookie|set-cookie|x-api-key|x-goog-api-key|api-key)$/i;

function authLabel(snapshot: Extract<LockedSnapshot, { providerMetadataStatus: "resolved" }>) {
  if (snapshot.authPlacement === "none") return "无认证";
  if (snapshot.authPlacement === "bearer_header") {
    return `${snapshot.authHeaderName ?? "Authorization"}: Bearer …`;
  }
  if (snapshot.authPlacement === "api_key_header") {
    return `${snapshot.authHeaderName ?? "API-Key"}: API Key …`;
  }
  if (snapshot.authPlacement === "query_param") {
    return `${snapshot.authHeaderName ?? "key"} 查询参数: API Key …`;
  }
  return "认证元数据未记录";
}

function parameterValue(value: unknown) {
  if (typeof value === "string") return value;
  const serialized = JSON.stringify(value);
  return serialized ?? String(value);
}

export function ContextInspector({
  open,
  items,
  rawItems = items,
  draftVersion = 0,
  appliedCheckpoint = null,
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
  const shownItems = tab === "snapshot"
    ? snapshot?.items ?? []
    : tab === "raw"
      ? rawItems
      : items;
  const itemsReadOnly = tab === "snapshot";
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
  const visibleAdditionalHeaders = snapshot
    ? Object.entries(snapshot.additionalHeaders)
        .filter(([name]) => !sensitiveHeaderPattern.test(name))
        .sort(([left], [right]) => left.localeCompare(right))
    : [];
  const effectiveParameters = snapshot
    ? Object.entries(snapshot.parameters).sort(([left], [right]) => left.localeCompare(right))
    : [];

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
        <p className="context-inspector__draft-version">草稿版本 v{draftVersion}</p>
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
        {tab === "raw" ? (
          <p className="context-inspector__path-note">
            原始内容只读；检查点不会删除历史内容，下方操作只调整下一轮 Context 草稿。
          </p>
        ) : null}

        {tab === "next" && appliedCheckpoint ? (
          <section className="context-inspector__checkpoint" aria-label="已应用 Context 检查点">
            <span>
              {appliedCheckpoint.kind === "compaction" ? "压缩检查点" : "分支摘要"}
              {" · "}
              {appliedCheckpoint.sourceRunIds.length} 个来源 Run
            </span>
            <strong>{appliedCheckpoint.summary}</strong>
            <dl>
              <div>
                <dt>保留边界</dt>
                <dd>{appliedCheckpoint.firstKeptRunId ?? "不保留尾部"}</dd>
              </div>
              <div>
                <dt>来源 Hash</dt>
                <dd>{appliedCheckpoint.sourceHash}</dd>
              </div>
              <div>
                <dt>摘要来源</dt>
                <dd>
                  {appliedCheckpoint.provider
                    ? `${appliedCheckpoint.provider.providerName} · ${appliedCheckpoint.provider.model}`
                    : "人工摘要"}
                </dd>
              </div>
            </dl>
          </section>
        ) : null}

        {tab === "snapshot" && (
          <div className="context-inspector__snapshot-heading">
            {snapshotLoading ? (
              <span>正在读取不可变凭证…</span>
            ) : snapshot ? (
              <>
                <span><FileCheck2 size={14} /> {dateTime(snapshot.createdAt)} 锁定</span>
                <code>{snapshot.canonicalHash}</code>
                {snapshot.providerMetadataStatus === "legacy" ? (
                  <p className="context-inspector__legacy-provider">
                    旧版快照：Provider 协议与认证元数据未记录
                  </p>
                ) : (
                  <div className="context-inspector__receipt-details">
                    <dl aria-label="Provider 快照">
                      <div>
                        <dt>模板</dt>
                        <dd>{snapshot.providerId} r{snapshot.templateRevision}</dd>
                      </div>
                      <div>
                        <dt>流协议</dt>
                        <dd>{protocolLabels[snapshot.streamProtocol]}</dd>
                      </div>
                      <div>
                        <dt>认证</dt>
                        <dd>{authLabel(snapshot)}</dd>
                      </div>
                    </dl>

                    {visibleAdditionalHeaders.length > 0 && (
                      <section aria-label="静态请求头">
                        <strong>静态请求头</strong>
                        <dl>
                          {visibleAdditionalHeaders.map(([name, value]) => (
                            <div key={name}>
                              <dt>{name}</dt>
                              <dd>{value}</dd>
                            </div>
                          ))}
                        </dl>
                      </section>
                    )}

                  </div>
                )}
                {effectiveParameters.length > 0 && (
                  <div className="context-inspector__receipt-details">
                    <section aria-label="有效模型参数">
                      <strong>有效模型参数</strong>
                      <dl>
                        {effectiveParameters.map(([name, value]) => (
                          <div key={name}>
                            <dt>{name}</dt>
                            <dd>{parameterValue(value)}</dd>
                          </div>
                        ))}
                      </dl>
                    </section>
                  </div>
                )}
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
                key={`${tab}-${item.id}`}
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
                    disabled={itemsReadOnly || item.mandatory}
                    className={item.pinned ? "is-active is-pin" : ""}
                    onClick={() => onTogglePinned(item)}
                  >
                    <Pin size={13} />
                  </button>
                  <button
                    type="button"
                    aria-label={`${item.included ? "排除" : "重新纳入"} ${item.label}`}
                    aria-pressed={item.included}
                    disabled={itemsReadOnly || item.mandatory}
                    className={item.included ? "is-active" : ""}
                    onClick={() => onToggleIncluded(item)}
                  >
                    {item.included ? <Eye size={13} /> : <EyeOff size={13} />}
                  </button>
                  {item.mandatory ? <span className="context-item__mandatory">必须纳入</span> : null}
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
