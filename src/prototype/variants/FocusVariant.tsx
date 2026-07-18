import {
  ArrowUp,
  BookOpen,
  Check,
  ChevronRight,
  CirclePlus,
  Copy,
  Eye,
  EyeOff,
  FileText,
  GitBranch,
  LoaderCircle,
  PanelRight,
  Paperclip,
  Pin,
  Route,
  Search,
  Send,
  Sparkles,
  X,
} from "lucide-react";
import { useMemo, useState } from "react";
import type { FormEvent } from "react";
import {
  contextItems,
  turns,
  workspace,
  workspaces,
  type ContextItem,
} from "../data";
import {
  Brand,
  Button,
  EndpointBadge,
  LocalBadge,
  MenuButton,
  ModelSelect,
  SavedState,
  StatusDot,
} from "../components/Shared";

type InspectorItem = ContextItem & { pinned: boolean };
type InspectorTab = "next" | "snapshot";

const routeTurnIds = new Set(["root", "focus", "context"]);

const initialSelectedRuns = Object.fromEntries(
  turns.map((turn) => [turn.id, turn.selectedRunId]),
) as Record<string, string>;

export function FocusVariant() {
  const routeTurns = useMemo(
    () => turns.filter((turn) => routeTurnIds.has(turn.id)),
    [],
  );
  const [activeWorkspace, setActiveWorkspace] = useState(workspace.name);
  const [activeTurnId, setActiveTurnId] = useState("context");
  const [selectedRuns, setSelectedRuns] =
    useState<Record<string, string>>(initialSelectedRuns);
  const [branchTurnId, setBranchTurnId] = useState<string | null>(null);
  const [branchDraft, setBranchDraft] = useState("");
  const [inspectorOpen, setInspectorOpen] = useState(true);
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>("next");
  const [inspectorItems, setInspectorItems] = useState<InspectorItem[]>(() =>
    contextItems.map((item) => ({
      ...item,
      pinned: item.kind === "pinned",
    })),
  );
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [toast, setToast] = useState<string | null>(null);

  const includedItems = inspectorItems.filter((item) => item.included);
  const includedTokens = includedItems.reduce((sum, item) => sum + item.tokens, 0);
  const pinnedCount = inspectorItems.filter((item) => item.pinned).length;
  const excludedCount = inspectorItems.length - includedItems.length;

  const showToast = (message: string) => {
    setToast(message);
    window.setTimeout(() => {
      setToast((current) => (current === message ? null : current));
    }, 2400);
  };

  const scrollToTurn = (turnId: string) => {
    setActiveTurnId(turnId);
    document
      .getElementById(`focus-turn-${turnId}`)
      ?.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  const selectRun = (turnId: string, runId: string, label: string) => {
    setSelectedRuns((current) => ({ ...current, [turnId]: runId }));
    showToast(`已切换到${label}，后续分支将绑定此版本`);
  };

  const toggleContextIncluded = (itemId: string) => {
    setInspectorItems((current) =>
      current.map((item) =>
        item.id === itemId ? { ...item, included: !item.included } : item,
      ),
    );
  };

  const toggleContextPinned = (itemId: string) => {
    setInspectorItems((current) =>
      current.map((item) =>
        item.id === itemId ? { ...item, pinned: !item.pinned } : item,
      ),
    );
  };

  const submitBranch = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!branchDraft.trim() || !branchTurnId) return;

    const sourceTurn = routeTurns.find((turn) => turn.id === branchTurnId);
    const selectedRunId = selectedRuns[branchTurnId] ?? sourceTurn?.selectedRunId;
    const selectedRun = sourceTurn?.runs.find((run) => run.id === selectedRunId);
    showToast(
      `已从「${sourceTurn?.title ?? "当前回答"} · ${selectedRun?.label ?? "所选版本"}」创建分支`,
    );
    setBranchTurnId(null);
    setBranchDraft("");
  };

  const submitMessage = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!draft.trim() || sending) return;

    setSending(true);
    showToast(`正在携带 ${includedItems.length} 项上下文发送`);
    window.setTimeout(() => {
      setSending(false);
      setDraft("");
      showToast("新 Turn 已加入当前路线");
    }, 1100);
  };

  return (
    <div
      className={`focus-variant ${inspectorOpen ? "focus-variant--inspector-open" : "focus-variant--inspector-closed"}`}
    >
      <aside className="focus-sidebar" aria-label="工作区与路线导航">
        <div className="focus-sidebar__brand-row">
          <Brand />
          <button type="button" className="icon-button" aria-label="搜索">
            <Search size={17} />
          </button>
        </div>

        <Button
          className="focus-sidebar__new-button"
          icon={<CirclePlus size={16} />}
          onClick={() => showToast("已准备一个空白对话")}
        >
          新建对话
        </Button>

        <nav className="focus-sidebar__section" aria-label="工作区">
          <div className="focus-sidebar__section-heading">
            <span>工作区</span>
            <button type="button" className="icon-button" aria-label="添加工作区">
              <CirclePlus size={14} />
            </button>
          </div>
          <div className="focus-workspace-list">
            {workspaces.map((item) => (
              <button
                key={item.name}
                type="button"
                className={`focus-workspace-list__item ${activeWorkspace === item.name ? "is-active" : ""}`}
                onClick={() => {
                  setActiveWorkspace(item.name);
                  if (item.name !== workspace.name) {
                    showToast(`原型中已预览「${item.name}」工作区入口`);
                  }
                }}
              >
                <span className="focus-workspace-list__icon">
                  <BookOpen size={14} />
                </span>
                <span className="focus-workspace-list__name">{item.name}</span>
                <span className="focus-workspace-list__count">{item.count}</span>
              </button>
            ))}
          </div>
        </nav>

        <nav className="focus-sidebar__section focus-route-nav" aria-label="当前路线">
          <div className="focus-sidebar__section-heading">
            <span>当前路线</span>
            <span className="focus-sidebar__section-meta">3 / 18</span>
          </div>
          <ol className="focus-route-nav__list">
            {routeTurns.map((turn, index) => (
              <li key={turn.id} className="focus-route-nav__step">
                <button
                  type="button"
                  className={`focus-route-nav__button ${activeTurnId === turn.id ? "is-active" : ""}`}
                  onClick={() => scrollToTurn(turn.id)}
                >
                  <span className="focus-route-nav__marker">{index + 1}</span>
                  <span className="focus-route-nav__copy">
                    <span className="focus-route-nav__label">{turn.branchLabel}</span>
                    <span className="focus-route-nav__title">{turn.title}</span>
                  </span>
                  {activeTurnId === turn.id && <ChevronRight size={14} />}
                </button>
              </li>
            ))}
          </ol>

          <div className="focus-route-nav__forks">
            <span className="focus-route-nav__forks-label">相邻分支</span>
            <button type="button" onClick={() => showToast("路线图将定位到「画布优先」")}>
              <GitBranch size={13} /> 画布优先
            </button>
            <button type="button" onClick={() => showToast("路线图将定位到「本地优先」")}>
              <GitBranch size={13} /> 本地优先
            </button>
          </div>
        </nav>

        <div className="focus-sidebar__footer">
          <LocalBadge />
          <span>{workspace.saveState}</span>
        </div>
      </aside>

      <main className="focus-main">
        <header className="focus-header">
          <div className="focus-header__identity">
            <div className="focus-header__eyebrow">
              <Route size={13} /> 当前路线 · 上下文透明
            </div>
            <h1>{workspace.name}</h1>
            <p>{workspace.goal}</p>
          </div>
          <div className="focus-header__actions">
            <SavedState />
            <Button
              icon={<Route size={15} />}
              onClick={() => showToast("路线图视图可在方向二中体验")}
            >
              路线图
            </Button>
            <Button
              active={inspectorOpen}
              icon={<PanelRight size={15} />}
              onClick={() => setInspectorOpen((current) => !current)}
            >
              上下文
              <span className="focus-header__context-count">{includedItems.length}</span>
            </Button>
            <MenuButton />
          </div>
        </header>

        <div className="focus-reading-pane">
          <div className="focus-reading-pane__intro">
            <div>
              <span className="focus-reading-pane__kicker">专注路线</span>
              <h2>从问题走到可验证的产品决策</h2>
            </div>
            <p>只展示当前祖先链；旁支留在路线导航中，不会混入本轮上下文。</p>
          </div>

          <div className="focus-thread" aria-label="当前路线的连续对话">
            {routeTurns.map((turn, turnIndex) => {
              const selectedRunId = selectedRuns[turn.id] ?? turn.selectedRunId;
              const selectedRun =
                turn.runs.find((run) => run.id === selectedRunId) ?? turn.runs[0];
              if (!selectedRun) return null;
              const isLocal = selectedRun.provider.toLowerCase().includes("ollama");

              return (
                <article
                  id={`focus-turn-${turn.id}`}
                  key={turn.id}
                  className={`focus-turn focus-turn--${turn.tone}`}
                  onMouseEnter={() => setActiveTurnId(turn.id)}
                >
                  <div className="focus-turn__rail" aria-hidden="true">
                    <span>{turnIndex + 1}</span>
                  </div>

                  <section className="focus-message focus-message--user">
                    <div className="focus-message__meta">
                      <span>你</span>
                      <time>{turn.createdAt}</time>
                      <span className="focus-message__branch-label">{turn.branchLabel}</span>
                    </div>
                    <p>{turn.prompt}</p>
                  </section>

                  <section className="focus-message focus-message--assistant">
                    <div className="focus-message__assistant-header">
                      <div className="focus-message__assistant-identity">
                        <span className="focus-message__assistant-mark">
                          <Sparkles size={14} />
                        </span>
                        <span>Flow 回答</span>
                        <EndpointBadge local={isLocal} />
                      </div>
                      <span className="focus-message__run-time">
                        {selectedRun.duration} · {selectedRun.usage}
                      </span>
                    </div>

                    {turn.runs.length > 1 && (
                      <div className="focus-run-switcher" aria-label={`${turn.title}的回答版本`}>
                        {turn.runs.map((run) => (
                          <button
                            key={run.id}
                            type="button"
                            className={`focus-run-switcher__option ${run.id === selectedRun.id ? "is-active" : ""}`}
                            aria-pressed={run.id === selectedRun.id}
                            onClick={() => selectRun(turn.id, run.id, run.label)}
                          >
                            <StatusDot status={run.status} />
                            <span className="focus-run-switcher__label">{run.label}</span>
                            <span className="focus-run-switcher__model">{run.model}</span>
                            {run.id === selectedRun.id && <Check size={13} />}
                          </button>
                        ))}
                      </div>
                    )}

                    {selectedRun.status === "interrupted" && (
                      <div className="focus-message__interrupted-note">
                        <span>本次生成已中断，已保留现有内容。</span>
                        <button
                          type="button"
                          onClick={() => showToast("已从 643 字处恢复生成")}
                        >
                          继续生成
                        </button>
                      </div>
                    )}

                    <div className="focus-message__answer-copy">
                      <p>{selectedRun.output}</p>
                    </div>

                    <div className="focus-message__actions">
                      <button
                        type="button"
                        onClick={() => showToast("回答已复制")}
                      >
                        <Copy size={14} /> 复制
                      </button>
                      <button
                        type="button"
                        className={branchTurnId === turn.id ? "is-active" : ""}
                        onClick={() => {
                          setBranchTurnId((current) => (current === turn.id ? null : turn.id));
                          setBranchDraft("");
                        }}
                      >
                        <GitBranch size={14} /> 从此回答分支
                      </button>
                      <button
                        type="button"
                        onClick={() => showToast("这条结论已固定到工作区")}
                      >
                        <Pin size={14} /> 固定结论
                      </button>
                    </div>

                    {branchTurnId === turn.id && (
                      <form className="focus-inline-branch" onSubmit={submitBranch}>
                        <div className="focus-inline-branch__source">
                          <GitBranch size={14} />
                          新分支将精确绑定「{selectedRun.label} · {selectedRun.model}」
                        </div>
                        <textarea
                          autoFocus
                          rows={2}
                          value={branchDraft}
                          onChange={(event) => setBranchDraft(event.target.value)}
                          placeholder="追问一个不同方向，例如：如果默认入口改成画布呢？"
                        />
                        <div className="focus-inline-branch__footer">
                          <span>旁支不会进入当前路线的下一次发送</span>
                          <button
                            type="submit"
                            className="focus-inline-branch__submit"
                            disabled={!branchDraft.trim()}
                          >
                            创建分支 <ArrowUp size={14} />
                          </button>
                        </div>
                      </form>
                    )}
                  </section>
                </article>
              );
            })}
          </div>
        </div>

        <form className="focus-composer" onSubmit={submitMessage}>
          <div className="focus-composer__scope">
            <span>
              <GitBranch size={13} /> 将从「上下文透明 · 回答 A」继续
            </span>
            <button type="button" onClick={() => setInspectorOpen(true)}>
              <FileText size={13} /> {includedItems.length} 项上下文 · 约{" "}
              {includedTokens.toLocaleString()} tokens
            </button>
          </div>
          <textarea
            rows={3}
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            placeholder="沿当前路线继续，或从上方任意回答创建旁支…"
            aria-label="发送新问题"
          />
          <div className="focus-composer__toolbar">
            <div className="focus-composer__tools">
              <button type="button" className="icon-button" aria-label="添加附件">
                <Paperclip size={17} />
              </button>
              <ModelSelect />
              <EndpointBadge />
            </div>
            <button
              type="submit"
              className="focus-composer__send"
              disabled={!draft.trim() || sending}
            >
              {sending ? <LoaderCircle className="is-spinning" size={16} /> : <Send size={16} />}
              {sending ? "发送中" : "发送"}
            </button>
          </div>
        </form>
      </main>

      {inspectorOpen && (
        <aside className="focus-inspector" aria-label="Context Inspector">
          <header className="focus-inspector__header">
            <div>
              <span className="focus-inspector__eyebrow">发送前凭证</span>
              <h2>Context Inspector</h2>
            </div>
            <button
              type="button"
              className="icon-button"
              aria-label="关闭上下文检查器"
              onClick={() => setInspectorOpen(false)}
            >
              <X size={17} />
            </button>
          </header>

          <div className="focus-inspector__summary">
            <div className="focus-inspector__summary-primary">
              <span>{includedItems.length} 项将发送</span>
              <strong>约 {includedTokens.toLocaleString()} tokens</strong>
            </div>
            <div className="focus-inspector__summary-detail">
              <span><Pin size={12} /> {pinnedCount} 项固定</span>
              <span><EyeOff size={12} /> {excludedCount} 项排除</span>
            </div>
          </div>

          <div className="focus-inspector__tabs" role="tablist" aria-label="上下文视图">
            <button
              type="button"
              role="tab"
              aria-selected={inspectorTab === "next"}
              className={inspectorTab === "next" ? "is-active" : ""}
              onClick={() => setInspectorTab("next")}
            >
              下次发送
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={inspectorTab === "snapshot"}
              className={inspectorTab === "snapshot" ? "is-active" : ""}
              onClick={() => setInspectorTab("snapshot")}
            >
              上次快照
            </button>
          </div>

          {inspectorTab === "snapshot" && (
            <div className="focus-inspector__snapshot-note">
              <Check size={14} /> 这是回答 A 生成时锁定的只读凭证
            </div>
          )}

          <div className="focus-context-list">
            {inspectorItems.map((item) => (
              <article
                key={item.id}
                className={`focus-context-item ${item.included ? "is-included" : "is-excluded"} ${item.pinned ? "is-pinned" : ""}`}
              >
                <div className="focus-context-item__ordinal">{item.ordinal}</div>
                <div className="focus-context-item__body">
                  <div className="focus-context-item__heading">
                    <span>{item.label}</span>
                    <span>{item.tokens} tok</span>
                  </div>
                  <span className="focus-context-item__source">{item.source}</span>
                  <p>{item.preview}</p>
                  <span className="focus-context-item__reason">{item.reason}</span>
                </div>
                <div className="focus-context-item__actions">
                  <button
                    type="button"
                    aria-label={item.pinned ? "取消固定" : "固定上下文"}
                    aria-pressed={item.pinned}
                    className={item.pinned ? "is-active" : ""}
                    disabled={inspectorTab === "snapshot"}
                    onClick={() => toggleContextPinned(item.id)}
                    title={item.pinned ? "取消固定" : "固定到后续发送"}
                  >
                    <Pin size={13} />
                  </button>
                  <button
                    type="button"
                    aria-label={item.included ? "从发送中排除" : "重新纳入发送"}
                    aria-pressed={item.included}
                    className={item.included ? "is-active" : ""}
                    disabled={inspectorTab === "snapshot"}
                    onClick={() => toggleContextIncluded(item.id)}
                    title={item.included ? "从发送中排除" : "重新纳入发送"}
                  >
                    {item.included ? <Eye size={13} /> : <EyeOff size={13} />}
                  </button>
                </div>
              </article>
            ))}
          </div>

          <footer className="focus-inspector__footer">
            <div className="focus-inspector__endpoint-row">
              <span>本轮发送至</span>
              <EndpointBadge />
            </div>
            <p>工作区保存在本机；纳入的上下文会随本轮请求发送至 OpenAI。</p>
          </footer>
        </aside>
      )}

      {toast && (
        <div className="focus-toast" role="status" aria-live="polite">
          <Check size={15} /> {toast}
        </div>
      )}
    </div>
  );
}
