import {
  ArrowUpRight,
  Check,
  ChevronRight,
  CircleDot,
  CornerDownRight,
  Eye,
  GitBranch,
  Layers3,
  LocateFixed,
  Map as MapIcon,
  Maximize2,
  Minus,
  PanelRight,
  Plus,
  Route,
  Search,
  Sparkles,
  X,
} from "lucide-react";
import { useMemo, useState } from "react";
import {
  contextItems,
  getSelectedRun,
  getTurn,
  turns,
  workspace,
  workspaces,
  type Turn,
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

type LensMode = "focus" | "context";

type DraftBranch = {
  id: string;
  title: string;
  parentId: string;
  parentRunId: string;
  parentRunLabel: string;
  parentModel: string;
  x: number;
  y: number;
};

type ReceiptItem = {
  id: string;
  ordinal: number;
  label: string;
  source: string;
  preview: string;
  tokens: number;
  included: boolean;
  reason: string;
};

const NODE_WIDTH = 220;
const NODE_HEIGHT = 108;
const byId = new Map(turns.map((turn) => [turn.id, turn]));

function getRouteTo(turnId: string) {
  const route: Turn[] = [];
  const visited = new Set<string>();
  let cursor = byId.get(turnId);

  while (cursor && !visited.has(cursor.id)) {
    route.unshift(cursor);
    visited.add(cursor.id);
    cursor = cursor.parentId ? byId.get(cursor.parentId) : undefined;
  }

  return route;
}

function connectorPath(parent: Pick<Turn, "x" | "y">, child: Pick<Turn, "x" | "y">) {
  if (child.x < parent.x + NODE_WIDTH + 32) {
    const sourceX = parent.x + NODE_WIDTH / 2;
    const targetX = child.x + NODE_WIDTH / 2;
    const travelsDown = child.y >= parent.y;
    const sourceY = travelsDown ? parent.y + NODE_HEIGHT : parent.y;
    const targetY = travelsDown ? child.y : child.y + NODE_HEIGHT;
    const middleY = (sourceY + targetY) / 2;

    return `M ${sourceX} ${sourceY} C ${sourceX} ${middleY}, ${targetX} ${middleY}, ${targetX} ${targetY}`;
  }

  const sourceX = parent.x + NODE_WIDTH;
  const sourceY = parent.y + NODE_HEIGHT / 2;
  const targetX = child.x;
  const targetY = child.y + NODE_HEIGHT / 2;
  const bend = Math.max(38, (targetX - sourceX) * 0.5);

  return `M ${sourceX} ${sourceY} C ${sourceX + bend} ${sourceY}, ${targetX - bend} ${targetY}, ${targetX} ${targetY}`;
}

function draftPosition(parent: Turn) {
  if (parent.x < 200) {
    return { x: 356, y: 342 };
  }

  return { x: 648, y: 580 };
}

export function CanvasVariant() {
  const [activeTurnId, setActiveTurnId] = useState("context");
  const [runSelections, setRunSelections] = useState<Record<string, string>>(() =>
    Object.fromEntries(turns.map((turn) => [turn.id, turn.selectedRunId])),
  );
  const [zoom, setZoom] = useState(0.9);
  const [lensMode, setLensMode] = useState<LensMode>("focus");
  const [branchOpen, setBranchOpen] = useState(false);
  const [branchPrompt, setBranchPrompt] = useState("");
  const [draftBranch, setDraftBranch] = useState<DraftBranch | null>(null);

  const activeTurn = getTurn(activeTurnId);
  const activeRun =
    activeTurn.runs.find((run) => run.id === runSelections[activeTurn.id]) ??
    getSelectedRun(activeTurn);
  const activeRoute = useMemo(() => getRouteTo(activeTurn.id), [activeTurn.id]);
  const activeRouteIds = useMemo(
    () => new Set(activeRoute.map((turn) => turn.id)),
    [activeRoute],
  );
  const isLocalRun = activeRun.provider.includes("本机") || activeRun.provider.includes("Ollama");

  const receiptItems = useMemo<ReceiptItem[]>(() => {
    if (activeTurn.id === "context") {
      return contextItems;
    }

    const routeItems: ReceiptItem[] = [
      {
        id: "dynamic-system",
        ordinal: 1,
        label: "系统说明",
        source: "工作区默认",
        preview: "你是一名严谨的 AI 产品设计顾问……",
        tokens: 186,
        included: true,
        reason: "工作区系统说明",
      },
    ];

    activeRoute.forEach((turn) => {
      const run =
        turn.runs.find((item) => item.id === runSelections[turn.id]) ?? getSelectedRun(turn);
      routeItems.push(
        {
          id: `${turn.id}-prompt`,
          ordinal: routeItems.length + 1,
          label: turn.branchLabel,
          source: turn.title,
          preview: turn.prompt,
          tokens: 40,
          included: true,
          reason: turn.id === activeTurn.id ? "当前问题" : "位于当前祖先路线",
        },
        {
          id: `${turn.id}-answer`,
          ordinal: routeItems.length + 2,
          label: `${run.label} · ${run.model}`,
          source: turn.title,
          preview: run.excerpt,
          tokens: 620,
          included: true,
          reason:
            turn.id === activeTurn.id
              ? "当前选中的回答版本"
              : "后续分支精确绑定到此回答",
        },
      );
    });

    return routeItems;
  }, [activeRoute, activeTurn.id, runSelections]);

  const estimatedTokens = receiptItems
    .filter((item) => item.included)
    .reduce((total, item) => total + item.tokens, 0);
  const excludedCount = receiptItems.filter((item) => !item.included).length;

  const selectTurn = (turnId: string) => {
    setActiveTurnId(turnId);
    setLensMode("focus");
    setBranchOpen(false);
  };

  const selectRun = (turnId: string, runId: string) => {
    setRunSelections((current) => ({ ...current, [turnId]: runId }));
    setActiveTurnId(turnId);
    setLensMode("focus");
  };

  const adjustZoom = (amount: number) => {
    setZoom((current) => Math.min(1.15, Math.max(0.62, Number((current + amount).toFixed(2)))));
  };

  const createBranch = () => {
    const title = branchPrompt.trim();
    if (!title) return;
    const position = draftPosition(activeTurn);

    setDraftBranch({
      id: `draft-${Date.now()}`,
      title,
      parentId: activeTurn.id,
      parentRunId: activeRun.id,
      parentRunLabel: activeRun.label,
      parentModel: activeRun.model,
      ...position,
    });
    setBranchPrompt("");
    setBranchOpen(false);
  };

  const draftParent = draftBranch ? byId.get(draftBranch.parentId) : undefined;

  return (
    <div className="canvas-variant">
      <header className="canvas-topbar">
        <div className="canvas-topbar__identity">
          <Brand />
          <span className="canvas-topbar__divider" aria-hidden="true" />
          <button type="button" className="canvas-workspace-title">
            <span>{workspace.name}</span>
            <span className="canvas-workspace-title__goal">{workspace.goal}</span>
          </button>
        </div>

        <div className="canvas-topbar__actions">
          <button type="button" className="canvas-command-search">
            <Search size={14} />
            <span>查找问题、结论或路径</span>
            <kbd>⌘ K</kbd>
          </button>
          <LocalBadge />
          <SavedState />
          <MenuButton label="工作区菜单" />
        </div>
      </header>

      <div className="canvas-layout">
        <aside className="map-index" aria-label="地图索引">
          <div className="map-index__header">
            <div>
              <span className="eyebrow">MAP INDEX</span>
              <h2>对话地图</h2>
            </div>
            <button type="button" className="icon-button" aria-label="定位当前节点">
              <LocateFixed size={16} />
            </button>
          </div>

          <div className="map-index__workspace-list">
            {workspaces.map((item) => (
              <button
                type="button"
                className={`map-index__workspace ${item.active ? "is-active" : ""}`}
                key={item.name}
              >
                <span>{item.name}</span>
                <span>{item.count}</span>
              </button>
            ))}
          </div>

          <nav className="map-index__route" aria-label="当前路线索引">
            <div className="map-index__section-label">
              <Route size={13} /> 当前路线 · {activeRoute.length} 层
            </div>
            {activeRoute.map((turn, index) => (
              <button
                type="button"
                className={`map-index__item ${turn.id === activeTurn.id ? "is-active" : ""}`}
                key={turn.id}
                onClick={() => selectTurn(turn.id)}
              >
                <span className="map-index__rail" aria-hidden="true">
                  <span>{String(index + 1).padStart(2, "0")}</span>
                </span>
                <span className="map-index__item-copy">
                  <strong>{turn.branchLabel}</strong>
                  <small>{turn.title}</small>
                </span>
                {turn.id === activeTurn.id && <CircleDot size={13} />}
              </button>
            ))}
          </nav>

          <div className="map-index__other">
            <div className="map-index__section-label">
              <Layers3 size={13} /> 旁支与搁置
            </div>
            {turns
              .filter((turn) => !activeRouteIds.has(turn.id))
              .map((turn) => (
                <button
                  type="button"
                  className="map-index__compact-item"
                  key={turn.id}
                  onClick={() => selectTurn(turn.id)}
                >
                  <span className={`map-index__tone map-index__tone--${turn.tone}`} />
                  <span>{turn.branchLabel}</span>
                  <small>{turn.branchCount || "—"}</small>
                </button>
              ))}
          </div>

          <div className="map-index__legend">
            <span><i className="is-current" /> 当前路径</span>
            <span><i className="is-branch" /> 旁支</span>
            <span><i className="is-muted" /> 已搁置</span>
          </div>
        </aside>

        <main className="canvas-workspace">
          <header className="canvas-board-header">
            <div className="canvas-board-header__title">
              <MapIcon size={15} />
              <div>
                <span>空间画布</span>
                <small>结构视图 · 回答内容在焦点镜头中展开</small>
              </div>
            </div>
            <div className="canvas-board-header__route">
              {activeRoute.map((turn, index) => (
                <span key={turn.id}>
                  {index > 0 && <ChevronRight size={12} />}
                  <button type="button" onClick={() => selectTurn(turn.id)}>
                    {turn.branchLabel}
                  </button>
                </span>
              ))}
            </div>
          </header>

          <section className="canvas-board" aria-label="分支对话画布">
            <div className="canvas-board__grid" aria-hidden="true" />

            <div className="canvas-toolbar" aria-label="画布缩放">
              <button type="button" onClick={() => adjustZoom(-0.08)} aria-label="缩小">
                <Minus size={15} />
              </button>
              <button type="button" className="canvas-toolbar__value" onClick={() => setZoom(0.9)}>
                {Math.round(zoom * 100)}%
              </button>
              <button type="button" onClick={() => adjustZoom(0.08)} aria-label="放大">
                <Plus size={15} />
              </button>
              <span />
              <button type="button" onClick={() => setZoom(0.9)} aria-label="适应视图">
                <Maximize2 size={15} />
              </button>
            </div>

            <div
              className="canvas-scene"
              style={{ transform: `scale(${zoom})` }}
            >
              <svg className="canvas-edges" viewBox="0 0 960 700" aria-hidden="true">
                <defs>
                  <marker
                    id="canvas-arrow"
                    viewBox="0 0 10 10"
                    refX="8"
                    refY="5"
                    markerWidth="5"
                    markerHeight="5"
                    orient="auto-start-reverse"
                  >
                    <path d="M 0 0 L 10 5 L 0 10 z" />
                  </marker>
                </defs>
                {turns.map((turn) => {
                  if (!turn.parentId) return null;
                  const parent = byId.get(turn.parentId);
                  if (!parent) return null;
                  const onRoute = activeRouteIds.has(parent.id) && activeRouteIds.has(turn.id);
                  const boundRun = parent.runs.find((run) => run.id === turn.parentRunId);
                  const labelX = (parent.x + NODE_WIDTH + turn.x) / 2;
                  const labelY = (parent.y + turn.y) / 2 + NODE_HEIGHT / 2 - 7;

                  return (
                    <g className={onRoute ? "is-route" : ""} key={`${parent.id}-${turn.id}`}>
                      <path d={connectorPath(parent, turn)} markerEnd="url(#canvas-arrow)" />
                      {boundRun && (
                        <text x={labelX} y={labelY}>
                          {boundRun.label.replace("回答 ", "")}
                        </text>
                      )}
                    </g>
                  );
                })}
                {draftBranch && draftParent && (
                  <g className="is-route is-draft">
                    <path d={connectorPath(draftParent, draftBranch)} markerEnd="url(#canvas-arrow)" />
                    <text
                      x={(draftParent.x + NODE_WIDTH + draftBranch.x) / 2}
                      y={(draftParent.y + draftBranch.y) / 2 + NODE_HEIGHT / 2 - 7}
                    >
                      {draftBranch.parentRunLabel.replace("回答 ", "")}
                    </text>
                  </g>
                )}
              </svg>

              {turns.map((turn, index) => {
                const selectedRun =
                  turn.runs.find((run) => run.id === runSelections[turn.id]) ??
                  getSelectedRun(turn);
                const isActive = turn.id === activeTurn.id;

                return (
                  <article
                    className={`canvas-node canvas-node--${turn.tone} ${isActive ? "is-active" : ""} ${activeRouteIds.has(turn.id) ? "is-route" : ""}`}
                    key={turn.id}
                    style={{ left: turn.x, top: turn.y }}
                  >
                    <button
                      type="button"
                      className="canvas-node__select"
                      onClick={() => selectTurn(turn.id)}
                    >
                      <span className="canvas-node__eyebrow">
                        <span>T-{String(index + 1).padStart(2, "0")}</span>
                        <span>{turn.branchLabel}</span>
                      </span>
                      <strong>{turn.title}</strong>
                      <span className="canvas-node__excerpt">{selectedRun.excerpt}</span>
                      <span className="canvas-node__meta">
                        <StatusDot status={selectedRun.status} />
                        {selectedRun.model}
                        <i />
                        {turn.branchCount} 个分支
                      </span>
                    </button>

                    {isActive && turn.runs.length > 1 && (
                      <div className="canvas-node__versions" aria-label="回答版本">
                        {turn.runs.map((run) => (
                          <button
                            type="button"
                            className={run.id === selectedRun.id ? "is-active" : ""}
                            key={run.id}
                            onClick={() => selectRun(turn.id, run.id)}
                          >
                            {run.label.replace("回答 ", "")}
                          </button>
                        ))}
                      </div>
                    )}
                  </article>
                );
              })}

              {draftBranch && (
                <article
                  className="canvas-node canvas-node--draft is-route"
                  style={{ left: draftBranch.x, top: draftBranch.y }}
                >
                  <div className="canvas-node__select">
                    <span className="canvas-node__eyebrow">
                      <span>NEW BRANCH</span>
                      <span>已创建</span>
                    </span>
                    <strong>{draftBranch.title}</strong>
                    <span className="canvas-node__excerpt">
                      精确继承 {draftBranch.parentRunLabel} · {draftBranch.parentModel}
                    </span>
                    <span className="canvas-node__meta canvas-node__meta--ready">
                      <Sparkles size={12} /> 等待发送
                    </span>
                  </div>
                </article>
              )}
            </div>

            <div className="canvas-coordinate" aria-hidden="true">
              X {activeTurn.x.toString().padStart(3, "0")} · Y {activeTurn.y.toString().padStart(3, "0")}
            </div>
          </section>

          <footer className={`context-receipt ${lensMode === "context" ? "is-open" : ""}`}>
            <div className="context-receipt__lead">
              <span className="context-receipt__icon"><Check size={14} /></span>
              <div>
                <strong>路径与上下文回执</strong>
                <span>
                  {receiptItems.filter((item) => item.included).length} 项将进入下一轮
                  {excludedCount > 0 ? ` · ${excludedCount} 项排除` : ""} · 约 {estimatedTokens.toLocaleString()} tokens
                </span>
              </div>
            </div>
            <div className="context-receipt__path">
              {activeRoute.map((turn, index) => (
                <span key={turn.id}>
                  {index > 0 && <ChevronRight size={11} />}
                  {turn.branchLabel}
                </span>
              ))}
            </div>
            <EndpointBadge local={isLocalRun} />
            <button
              type="button"
              className="context-receipt__inspect"
              onClick={() => setLensMode((mode) => (mode === "context" ? "focus" : "context"))}
            >
              <Eye size={14} />
              {lensMode === "context" ? "返回回答" : "检查明细"}
              <ChevronRight size={13} />
            </button>
          </footer>
        </main>

        <aside className="focus-lens" aria-label="焦点镜头">
          <header className="focus-lens__header">
            <div>
              <span className="eyebrow">FOCUS LENS</span>
              <h2>{lensMode === "focus" ? "焦点镜头" : "上下文明细"}</h2>
            </div>
            <button
              type="button"
              className="icon-button"
              aria-label="切换焦点镜头"
              onClick={() => setLensMode((mode) => (mode === "focus" ? "context" : "focus"))}
            >
              <PanelRight size={16} />
            </button>
          </header>

          {lensMode === "focus" ? (
            <div className="focus-lens__body">
              <div className="focus-lens__crumbs">
                {activeRoute.map((turn, index) => (
                  <span key={turn.id}>
                    {index > 0 && <ChevronRight size={11} />}
                    {turn.branchLabel}
                  </span>
                ))}
              </div>

              <section className="focus-question">
                <span className="focus-question__label">当前问题 · {activeTurn.createdAt}</span>
                <h3>{activeTurn.prompt}</h3>
              </section>

              <div className="focus-version-tabs" role="tablist" aria-label="回答版本">
                {activeTurn.runs.map((run) => (
                  <button
                    type="button"
                    role="tab"
                    aria-selected={run.id === activeRun.id}
                    className={run.id === activeRun.id ? "is-active" : ""}
                    key={run.id}
                    onClick={() => selectRun(activeTurn.id, run.id)}
                  >
                    <span>{run.label}</span>
                    <small>{run.model}</small>
                  </button>
                ))}
                <button type="button" className="focus-version-tabs__add" aria-label="生成另一个回答">
                  <Plus size={15} />
                </button>
              </div>

              <article className="focus-answer">
                <header className="focus-answer__meta">
                  <div>
                    <StatusDot status={activeRun.status} />
                    <strong>{activeRun.model}</strong>
                    <span>{activeRun.provider}</span>
                  </div>
                  <span>{activeRun.duration} · {activeRun.usage}</span>
                </header>
                <p>{activeRun.output}</p>
                <div className="focus-answer__signal">
                  <CornerDownRight size={14} />
                  <span>后续分支会记录并锁定到此回答版本</span>
                </div>
              </article>

              <div className="focus-actions">
                <Button
                  className="focus-actions__branch"
                  icon={<GitBranch size={15} />}
                  active={branchOpen}
                  onClick={() => setBranchOpen((open) => !open)}
                >
                  从此回答分支
                </Button>
                <button type="button" className="focus-actions__link">
                  <ArrowUpRight size={14} /> 引用到另一条路线
                </button>
              </div>

              {branchOpen && (
                <section className="branch-composer">
                  <header>
                    <div>
                      <GitBranch size={14} />
                      <strong>新分支</strong>
                    </div>
                    <button type="button" onClick={() => setBranchOpen(false)} aria-label="关闭分支输入框">
                      <X size={14} />
                    </button>
                  </header>
                  <div className="branch-composer__binding">
                    <span>精确来源</span>
                    <strong>{activeTurn.branchLabel} / {activeRun.label}</strong>
                    <small>{activeRun.model} · {activeRun.id}</small>
                  </div>
                  <textarea
                    value={branchPrompt}
                    onChange={(event) => setBranchPrompt(event.target.value)}
                    placeholder="沿着这个回答，继续追问一个更具体的问题……"
                    autoFocus
                  />
                  <footer>
                    <ModelSelect local={isLocalRun} />
                    <button
                      type="button"
                      className="branch-composer__submit"
                      disabled={!branchPrompt.trim()}
                      onClick={createBranch}
                    >
                      创建分支 <GitBranch size={14} />
                    </button>
                  </footer>
                </section>
              )}

              {draftBranch && (
                <div className="branch-created" role="status">
                  <Check size={14} />
                  <div>
                    <strong>分支已落在画布上</strong>
                    <span>{draftBranch.parentRunLabel} · {draftBranch.parentModel} 已写入来源</span>
                  </div>
                  <button type="button" onClick={() => setDraftBranch(null)} aria-label="移除演示分支">
                    <X size={13} />
                  </button>
                </div>
              )}
            </div>
          ) : (
            <div className="context-inspector">
              <div className="context-inspector__summary">
                <span className="context-inspector__stamp"><Check size={16} /></span>
                <div>
                  <strong>下一次发送凭证</strong>
                  <span>{activeRun.model} · {estimatedTokens.toLocaleString()} tokens</span>
                </div>
                <EndpointBadge local={isLocalRun} />
              </div>

              <div className="context-inspector__notice">
                <Eye size={14} />
                这是有序清单。旁支不会因为画布位置接近而自动进入上下文。
              </div>

              <ol className="context-inspector__list">
                {receiptItems.map((item) => (
                  <li className={item.included ? "is-included" : "is-excluded"} key={item.id}>
                    <span className="context-inspector__ordinal">
                      {String(item.ordinal).padStart(2, "0")}
                    </span>
                    <div>
                      <div className="context-inspector__item-title">
                        <strong>{item.label}</strong>
                        <span>{item.tokens} t</span>
                      </div>
                      <small>{item.source}</small>
                      <p>{item.preview}</p>
                      <span className="context-inspector__reason">
                        {item.included ? <Check size={11} /> : <X size={11} />}
                        {item.reason}
                      </span>
                    </div>
                  </li>
                ))}
              </ol>

              <button type="button" className="context-inspector__back" onClick={() => setLensMode("focus")}>
                返回 {activeRun.label} <ChevronRight size={14} />
              </button>
            </div>
          )}
        </aside>
      </div>
    </div>
  );
}
