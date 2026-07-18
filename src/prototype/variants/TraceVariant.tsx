import {
  Activity,
  Check,
  CheckCircle2,
  ChevronRight,
  Circle,
  Clock3,
  Cloud,
  Eye,
  FileText,
  Fingerprint,
  HardDrive,
  History,
  Info,
  ListOrdered,
  Lock,
  MessageSquare,
  MoreHorizontal,
  Pin,
  Route,
  Send,
  Server,
  ShieldCheck,
  X,
} from "lucide-react";
import { useMemo, useState } from "react";
import {
  contextItems,
  getSelectedRun,
  getTurn,
  selectedTurnId as initialTurnId,
  workspace,
} from "../data";
import {
  Brand,
  LocalBadge,
  SavedState,
  StatusDot,
} from "../components/Shared";

type EvidenceTab = "next" | "history";
type ProviderMode = "local" | "cloud";

type RunEvent = {
  id: string;
  time: string;
  label: string;
  state: "complete" | "verified";
  summary: string;
  detail: string;
  fields: Array<{ label: string; value: string }>;
};

const routeTurnIds = ["root", "focus", "context"];
const routeTurns = routeTurnIds.map((id) => getTurn(id));

const runEvents: RunEvent[] = [
  {
    id: "assemble",
    time: "21:48:06.014",
    label: "上下文已组装",
    state: "verified",
    summary: "按祖先路线排序，固定项插入当前问题之前。",
    detail:
      "运行器从选中的回答版本回溯到根问题，再附加手动固定项与当前输入。被排除的画布假设没有进入请求体。",
    fields: [
      { label: "策略", value: "ancestor-path@2" },
      { label: "输入项", value: "7 included · 1 excluded" },
      { label: "估算", value: "2,264 tokens" },
    ],
  },
  {
    id: "boundary",
    time: "21:48:06.021",
    label: "发送边界已确认",
    state: "verified",
    summary: "请求发往 OpenAI；工作区文件仍保存在本机。",
    detail:
      "仅本次请求清单与模型参数离开设备。路线、旁支、注释和历史快照由本地工作区保存。",
    fields: [
      { label: "Provider", value: "OpenAI" },
      { label: "Model", value: "GPT-4.1" },
      { label: "Base URL", value: "api.openai.com/v1" },
    ],
  },
  {
    id: "response",
    time: "21:48:52.487",
    label: "回答已封存",
    state: "complete",
    summary: "完成 1,204 tokens；凭证与回答版本绑定。",
    detail:
      "流式输出完成后，发送清单、端点、模型参数与回答内容共同写入不可变运行快照，可用于之后的差异检查。",
    fields: [
      { label: "耗时", value: "46.4 s" },
      { label: "输出", value: "1,204 tokens" },
      { label: "快照", value: "trace_0247.json" },
    ],
  },
];

function contextKindLabel(kind: (typeof contextItems)[number]["kind"]) {
  switch (kind) {
    case "system":
      return "系统";
    case "prompt":
      return "问题";
    case "answer":
      return "回答";
    case "pinned":
      return "固定";
    case "current":
      return "当前";
  }
}

function ContextKindIcon({ kind }: { kind: (typeof contextItems)[number]["kind"] }) {
  if (kind === "pinned") return <Pin size={13} />;
  if (kind === "system") return <ShieldCheck size={13} />;
  if (kind === "current") return <MessageSquare size={13} />;
  return <FileText size={13} />;
}

export function TraceVariant() {
  const [activeTurnId, setActiveTurnId] = useState(initialTurnId);
  const [evidenceTab, setEvidenceTab] = useState<EvidenceTab>("next");
  const [provider, setProvider] = useState<ProviderMode>("local");
  const [selectedEventId, setSelectedEventId] = useState(runEvents[0].id);
  const [includedIds, setIncludedIds] = useState<Set<string>>(
    () => new Set(contextItems.filter((item) => item.included).map((item) => item.id)),
  );
  const [sealed, setSealed] = useState(false);

  const includedCount = includedIds.size;
  const excludedCount = contextItems.length - includedCount;
  const includedTokens = useMemo(
    () =>
      contextItems.reduce(
        (total, item) => total + (includedIds.has(item.id) ? item.tokens : 0),
        0,
      ),
    [includedIds],
  );
  const selectedEvent =
    runEvents.find((event) => event.id === selectedEventId) ?? runEvents[0];

  const toggleContext = (id: string) => {
    setSealed(false);
    setIncludedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const changeProvider = (mode: ProviderMode) => {
    setSealed(false);
    setProvider(mode);
  };

  return (
    <div className="trace-variant">
      <header className="trace-header">
        <div className="trace-header__identity">
          <Brand />
          <span className="trace-header__rule" />
          <div className="trace-header__workspace">
            <span className="trace-eyebrow">工作区 / 研究路线</span>
            <strong>{workspace.name}</strong>
          </div>
        </div>

        <div className="trace-header__status">
          <LocalBadge />
          <SavedState />
          <button type="button" className="trace-icon-button" aria-label="更多工作区操作">
            <MoreHorizontal size={18} />
          </button>
        </div>
      </header>

      <div className="trace-shell">
        <aside className="trace-route" aria-label="当前路线大纲">
          <div className="trace-route__heading">
            <div>
              <span className="trace-eyebrow">Route 03</span>
              <h2>当前路线</h2>
            </div>
            <span className="trace-route__verified">
              <Check size={12} /> 可回溯
            </span>
          </div>

          <nav className="trace-route__steps">
            {routeTurns.map((turn, index) => {
              const selected = activeTurnId === turn.id;
              const run = getSelectedRun(turn);
              return (
                <button
                  type="button"
                  className={`trace-route-step ${selected ? "is-active" : ""}`}
                  key={turn.id}
                  onClick={() => setActiveTurnId(turn.id)}
                  aria-current={selected ? "step" : undefined}
                >
                  <span className="trace-route-step__rail" aria-hidden="true">
                    <span className="trace-route-step__index">
                      {String(index + 1).padStart(2, "0")}
                    </span>
                  </span>
                  <span className="trace-route-step__body">
                    <span className="trace-route-step__type">
                      {turn.branchLabel}
                      <StatusDot status={run.status} />
                    </span>
                    <strong>{turn.title}</strong>
                    <small>{run.model} · {turn.createdAt}</small>
                  </span>
                  <ChevronRight size={14} className="trace-route-step__chevron" />
                </button>
              );
            })}
          </nav>

          <div className="trace-route__branch-note">
            <Route size={15} />
            <div>
              <strong>旁支未混入</strong>
              <p>“画布优先”与“本地优先”保留在工作区，但不属于本轮祖先路径。</p>
            </div>
          </div>

          <div className="trace-route__legend">
            <span><Circle size={7} fill="currentColor" /> 祖先项</span>
            <span><Pin size={11} /> 手动固定</span>
            <span><X size={11} /> 已排除</span>
          </div>
        </aside>

        <main className="trace-session">
          <div className="trace-session__heading">
            <div>
              <span className="trace-eyebrow">Session trace / tf-0247</span>
              <h1>让上下文边界成为对话的一部分</h1>
            </div>
            <div className="trace-session__assurance">
              <Fingerprint size={16} />
              <div>
                <span>路线签名</span>
                <code>9D2A…7C41</code>
              </div>
            </div>
          </div>

          <div className="trace-session__goal">
            <span>本次目标</span>
            <p>{workspace.goal}</p>
          </div>

          <section className="trace-transcript" aria-label="路线会话正文">
            {routeTurns.map((turn, index) => {
              const run = getSelectedRun(turn);
              const selected = activeTurnId === turn.id;
              return (
                <article
                  className={`trace-turn ${selected ? "is-active" : ""}`}
                  key={turn.id}
                  id={`trace-turn-${turn.id}`}
                  onClick={() => setActiveTurnId(turn.id)}
                >
                  <div className="trace-turn__meta">
                    <span className="trace-turn__number">TURN {String(index + 1).padStart(2, "0")}</span>
                    <span>{turn.createdAt}</span>
                    <span className="trace-turn__binding"><Lock size={11} /> 父回答已绑定</span>
                  </div>

                  <div className="trace-message trace-message--user">
                    <div className="trace-message__author">你</div>
                    <p>{turn.prompt}</p>
                  </div>

                  <div className="trace-message trace-message--assistant">
                    <div className="trace-message__author">
                      <span className="trace-model-mark" />
                      <span>{run.model}</span>
                      <span className="trace-message__provider">{run.provider}</span>
                      <StatusDot status={run.status} />
                    </div>
                    <p>{run.output}</p>
                    <div className="trace-message__receipt">
                      <span><Clock3 size={12} /> {run.duration}</span>
                      {run.usage && <span>{run.usage}</span>}
                      <button type="button" onClick={() => setEvidenceTab("history")}>
                        <Eye size={12} /> 查看运行凭证
                      </button>
                    </div>
                  </div>
                </article>
              );
            })}
          </section>

          <section className="trace-composer" aria-label="新一轮对话输入">
            <div className="trace-composer__context-line">
              <ShieldCheck size={14} />
              <span>下一次发送将使用右侧已核对清单</span>
              <strong>{includedCount} 项 · 约 {includedTokens.toLocaleString("zh-CN")} tokens</strong>
            </div>
            <div className="trace-composer__input">
              <textarea
                aria-label="输入后续问题"
                placeholder="沿当前路线继续提问…"
                defaultValue="请把发送凭证的渐进披露方案细化成可测试的交互。"
              />
              <div className="trace-composer__actions">
                <span>⌘ ↵ 发送</span>
                <button
                  type="button"
                  className="trace-composer__send"
                  onClick={() => setSealed(true)}
                >
                  <Send size={15} /> 按凭证发送
                </button>
              </div>
            </div>
          </section>
        </main>

        <aside className="trace-evidence" aria-label="发送凭证与运行记录">
          <div className="trace-evidence__fixed-head">
            <div className="trace-evidence__title-row">
              <div>
                <span className="trace-eyebrow">Context credential</span>
                <h2>发送凭证</h2>
              </div>
              <span className={`trace-ready-state ${sealed ? "is-sealed" : ""}`}>
                {sealed ? <Lock size={12} /> : <CheckCircle2 size={12} />}
                {sealed ? "已封存" : "待发送"}
              </span>
            </div>

            <div className="trace-credential-summary">
              <div>
                <span>内容边界</span>
                <strong>{includedCount} / {contextItems.length} 项</strong>
              </div>
              <div>
                <span>估算输入</span>
                <strong>{includedTokens.toLocaleString("zh-CN")}</strong>
              </div>
              <div>
                <span>排除</span>
                <strong>{excludedCount}</strong>
              </div>
            </div>

            <div className="trace-provider" aria-label="选择推理端点">
              <span className="trace-provider__label">本轮发送到</span>
              <div className="trace-provider__switch">
                <button
                  type="button"
                  className={provider === "local" ? "is-active" : ""}
                  onClick={() => changeProvider("local")}
                  aria-pressed={provider === "local"}
                >
                  <HardDrive size={13} /> 本机
                </button>
                <button
                  type="button"
                  className={provider === "cloud" ? "is-active" : ""}
                  onClick={() => changeProvider("cloud")}
                  aria-pressed={provider === "cloud"}
                >
                  <Cloud size={13} /> 云端
                </button>
              </div>
              <div className="trace-provider__endpoint">
                <Server size={13} />
                <span>
                  <strong>{provider === "local" ? "Qwen3:14b" : "GPT-4.1"}</strong>
                  {provider === "local" ? "localhost:11434" : "api.openai.com/v1"}
                </span>
              </div>
              {provider === "cloud" && (
                <p className="trace-provider__notice">
                  <Info size={12} /> 清单内容将离开设备；路线文件仍保存在本机。
                </p>
              )}
            </div>
          </div>

          <div className="trace-evidence__tabs" role="tablist" aria-label="凭证视图">
            <button
              type="button"
              role="tab"
              aria-selected={evidenceTab === "next"}
              className={evidenceTab === "next" ? "is-active" : ""}
              onClick={() => setEvidenceTab("next")}
            >
              <ListOrdered size={14} /> 下一次发送
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={evidenceTab === "history"}
              className={evidenceTab === "history" ? "is-active" : ""}
              onClick={() => setEvidenceTab("history")}
            >
              <History size={14} /> 历史快照
            </button>
          </div>

          <div className="trace-evidence__scroll">
            {evidenceTab === "next" ? (
              <section className="trace-manifest" aria-label="下一次发送有序清单">
                <div className="trace-section-label">
                  <span>ORDERED MANIFEST</span>
                  <small>点击任一项以包含 / 排除</small>
                </div>
                <ol className="trace-manifest__list">
                  {contextItems.map((item) => {
                    const included = includedIds.has(item.id);
                    return (
                      <li
                        className={`trace-manifest-item ${included ? "is-included" : "is-excluded"}`}
                        key={item.id}
                      >
                        <button
                          type="button"
                          className="trace-manifest-item__toggle"
                          onClick={() => toggleContext(item.id)}
                          aria-pressed={included}
                          aria-label={`${included ? "排除" : "包含"}${item.label}`}
                        >
                          <span className="trace-manifest-item__ordinal">
                            {String(item.ordinal).padStart(2, "0")}
                          </span>
                          <span className="trace-manifest-item__check">
                            {included ? <Check size={12} /> : <X size={12} />}
                          </span>
                        </button>
                        <div className="trace-manifest-item__content">
                          <div className="trace-manifest-item__meta">
                            <span className={`trace-kind trace-kind--${item.kind}`}>
                              <ContextKindIcon kind={item.kind} />
                              {contextKindLabel(item.kind)}
                            </span>
                            <span>{item.tokens} tkn</span>
                          </div>
                          <strong>{item.label}</strong>
                          <p>{item.preview}</p>
                          <small>{included ? item.reason : "不会进入下一次请求"}</small>
                        </div>
                      </li>
                    );
                  })}
                </ol>
              </section>
            ) : (
              <section className="trace-run" aria-label="历史运行快照">
                <div className="trace-run__snapshot-head">
                  <div>
                    <span className="trace-section-label">RUN SNAPSHOT</span>
                    <strong>tf-run-0247</strong>
                    <small>7 月 17 日 21:48 · 不可变</small>
                  </div>
                  <span className="trace-run__hash">
                    <Fingerprint size={13} /> 9D2A…7C41
                  </span>
                </div>

                <div className="trace-run__facts">
                  <span><Cloud size={12} /> OpenAI / GPT-4.1</span>
                  <span><FileText size={12} /> 7 项 / 2,264 tkn</span>
                  <span><Clock3 size={12} /> 46.4 s</span>
                </div>

                <div className="trace-section-label">
                  <span>RUN EVENTS</span>
                  <small>选择事件检查证据</small>
                </div>
                <div className="trace-run-events">
                  {runEvents.map((event) => (
                    <button
                      type="button"
                      key={event.id}
                      className={`trace-run-event ${selectedEventId === event.id ? "is-active" : ""}`}
                      onClick={() => setSelectedEventId(event.id)}
                    >
                      <span className="trace-run-event__line" aria-hidden="true">
                        {event.state === "verified" ? <ShieldCheck size={13} /> : <Check size={13} />}
                      </span>
                      <span className="trace-run-event__body">
                        <span><time>{event.time}</time>{event.label}</span>
                        <small>{event.summary}</small>
                      </span>
                      <ChevronRight size={13} />
                    </button>
                  ))}
                </div>

                <article className="trace-event-inspector" aria-live="polite">
                  <div className="trace-event-inspector__heading">
                    <Activity size={14} />
                    <strong>{selectedEvent.label}</strong>
                    <span>{selectedEvent.time}</span>
                  </div>
                  <p>{selectedEvent.detail}</p>
                  <dl>
                    {selectedEvent.fields.map((field) => (
                      <div key={field.label}>
                        <dt>{field.label}</dt>
                        <dd>{field.value}</dd>
                      </div>
                    ))}
                  </dl>
                </article>

                <details className="trace-snapshot-manifest">
                  <summary>
                    <ListOrdered size={13} /> 查看已封存清单
                    <span>7 项</span>
                  </summary>
                  <ol>
                    {contextItems.filter((item) => item.included).map((item) => (
                      <li key={item.id}>
                        <span>{String(item.ordinal).padStart(2, "0")}</span>
                        <strong>{item.label}</strong>
                        <small>{item.tokens} tkn</small>
                      </li>
                    ))}
                  </ol>
                </details>
              </section>
            )}
          </div>
        </aside>
      </div>
    </div>
  );
}
