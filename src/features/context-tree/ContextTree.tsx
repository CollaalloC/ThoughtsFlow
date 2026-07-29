import {
  Check,
  CircleDot,
  GitBranch,
  Pencil,
  Search,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent, KeyboardEvent } from "react";
import type {
  ContextBranchView,
  ContextCheckpointView,
  ContextTreeProjection,
} from "../../shared/contracts";
import "./context-tree.css";

export type ContextTreeProjectionView = ContextTreeProjection;

type ContextTreeProps = {
  projection: ContextTreeProjectionView;
  busy?: boolean;
  onSelect: (
    runId: string | null,
    branchId: string | null,
  ) => void | Promise<void>;
  onRenameBranch?: (
    branchId: string,
    name: string,
    expectedVersion: number,
  ) => void | Promise<void>;
};

export function resolveContextBranchId(
  projection: ContextTreeProjectionView,
  branchIds: string[],
) {
  const activeBranchId = projection.cursor.branchId;
  if (activeBranchId && branchIds.includes(activeBranchId)) {
    return activeBranchId;
  }
  return branchIds.length === 1 ? branchIds[0] : null;
}

const statusLabels: Record<string, string> = {
  queued: "等待中",
  pending: "等待中",
  connecting: "连接中",
  streaming: "生成中",
  completed: "已完成",
  failed: "失败",
  cancelled: "已取消",
  interrupted: "已中断",
};

function terms(value: string) {
  return value
    .trim()
    .toLocaleLowerCase()
    .split(/\s+/)
    .filter(Boolean);
}

function checkpointLabel(checkpoint: ContextCheckpointView) {
  const kind = checkpoint.kind === "compaction" ? "压缩检查点" : "分支摘要";
  return `${kind} · ${checkpoint.summary}`;
}

export function ContextTree({
  projection,
  busy = false,
  onSelect,
  onRenameBranch,
}: ContextTreeProps) {
  const [scope, setScope] = useState<"path" | "all">("path");
  const [query, setQuery] = useState("");
  const [editingBranchId, setEditingBranchId] = useState<string | null>(null);
  const [branchName, setBranchName] = useState("");
  const [focusedRunId, setFocusedRunId] = useState<string | null>(
    projection.cursor.activeRunId,
  );
  const treeItemRefs = useRef(new Map<string | null, HTMLButtonElement>());

  const branchById = useMemo(
    () => new Map(projection.branches.map((branch) => [branch.id, branch])),
    [projection.branches],
  );
  const checkpointById = useMemo(
    () => new Map(projection.checkpoints.map((checkpoint) => [checkpoint.id, checkpoint])),
    [projection.checkpoints],
  );
  const parentByRun = useMemo(
    () => new Map(projection.edges.map((edge) => [edge.targetRunId, edge.sourceRunId])),
    [projection.edges],
  );
  const depthByRun = useMemo(() => {
    const depths = new Map<string, number>();
    const depth = (runId: string, visited = new Set<string>()): number => {
      const known = depths.get(runId);
      if (known !== undefined) return known;
      if (visited.has(runId)) return 1;
      visited.add(runId);
      const parent = parentByRun.get(runId);
      const value = parent ? depth(parent, visited) + 1 : 1;
      depths.set(runId, value);
      return value;
    };
    projection.nodes.forEach((node) => depth(node.runId));
    return depths;
  }, [parentByRun, projection.nodes]);

  const visibleNodes = useMemo(() => {
    const searchTerms = terms(query);
    return projection.nodes.filter((node) => {
      if (scope === "path" && !node.isOnActivePath) return false;
      if (searchTerms.length === 0) return true;
      const branches = node.branchIds
        .map((branchId) => branchById.get(branchId)?.name ?? "")
        .join(" ");
      const haystack = [
        node.runId,
        node.title ?? "",
        node.prompt,
        node.outputPreview,
        node.model,
        node.status,
        statusLabels[node.status] ?? "",
        branches,
      ]
        .join(" ")
        .toLocaleLowerCase();
      return searchTerms.every((term) => haystack.includes(term));
    });
  }, [branchById, projection.nodes, query, scope]);
  const visibleTreeItemIds = useMemo<Array<string | null>>(
    () => [null, ...visibleNodes.map((node) => node.runId)],
    [visibleNodes],
  );

  useEffect(() => {
    if (visibleTreeItemIds.includes(focusedRunId)) return;
    const activeRunId = projection.cursor.activeRunId;
    setFocusedRunId(
      activeRunId && visibleTreeItemIds.includes(activeRunId) ? activeRunId : null,
    );
  }, [focusedRunId, projection.cursor.activeRunId, visibleTreeItemIds]);

  const registerTreeItem = (
    runId: string | null,
    element: HTMLButtonElement | null,
  ) => {
    if (element) {
      treeItemRefs.current.set(runId, element);
    } else {
      treeItemRefs.current.delete(runId);
    }
  };

  const focusTreeItem = (runId: string | null) => {
    setFocusedRunId(runId);
    treeItemRefs.current.get(runId)?.focus();
  };

  const handleTreeKeyDown = (
    event: KeyboardEvent<HTMLButtonElement>,
    runId: string | null,
  ) => {
    const currentIndex = visibleTreeItemIds.indexOf(runId);
    if (currentIndex < 0) return;

    let handled = true;
    let nextRunId = runId;
    switch (event.key) {
      case "ArrowDown":
        nextRunId =
          visibleTreeItemIds[Math.min(currentIndex + 1, visibleTreeItemIds.length - 1)];
        break;
      case "ArrowUp":
        nextRunId = visibleTreeItemIds[Math.max(currentIndex - 1, 0)];
        break;
      case "Home":
        nextRunId = visibleTreeItemIds[0];
        break;
      case "End":
        nextRunId = visibleTreeItemIds.at(-1) ?? null;
        break;
      case "ArrowLeft": {
        const parentRunId = runId ? (parentByRun.get(runId) ?? null) : null;
        nextRunId = visibleTreeItemIds.includes(parentRunId) ? parentRunId : null;
        break;
      }
      case "ArrowRight": {
        const child = visibleNodes.find((node) => node.parentRunId === runId);
        nextRunId = child?.runId ?? runId;
        break;
      }
      default:
        handled = false;
    }
    if (!handled) return;
    event.preventDefault();
    focusTreeItem(nextRunId);
  };

  const startRename = (branch: ContextBranchView) => {
    setEditingBranchId(branch.id);
    setBranchName(branch.name);
  };

  const submitRename = (event: FormEvent, branch: ContextBranchView) => {
    event.preventDefault();
    const name = branchName.trim();
    if (!name || name === branch.name) {
      setEditingBranchId(null);
      return;
    }
    void onRenameBranch?.(branch.id, name, branch.version);
    setEditingBranchId(null);
  };

  return (
    <section className="context-tree" aria-label="Context Tree">
      <header className="context-tree__header">
        <div>
          <span>CONTEXT TREE</span>
          <strong>精确 Model Run</strong>
        </div>
        <span className="context-tree__version">v{projection.cursor.version}</span>
      </header>

      <div className="context-tree__tools">
        <div className="context-tree__scope" role="group" aria-label="Context Tree 范围">
          <button
            aria-pressed={scope === "path"}
            onClick={() => setScope("path")}
            type="button"
          >
            当前路径
          </button>
          <button
            aria-pressed={scope === "all"}
            onClick={() => setScope("all")}
            type="button"
          >
            全部节点
          </button>
        </div>
        <label className="context-tree__search">
          <Search aria-hidden="true" size={13} />
          <input
            aria-label="搜索 Context Tree"
            onChange={(event) => setQuery(event.target.value)}
            placeholder="问题、回答、模型或状态"
            type="search"
            value={query}
          />
        </label>
      </div>

      <div className="context-tree__body" role="tree" aria-label="上下文运行树">
        <div className="context-tree__row is-root">
          <button
            aria-current={projection.cursor.activeRunId === null ? "true" : undefined}
            aria-label={`工作区起点${projection.cursor.activeRunId === null ? " · 当前 Context" : ""}`}
            aria-level={1}
            className={projection.cursor.activeRunId === null ? "is-active" : ""}
            disabled={busy}
            onFocus={() => setFocusedRunId(null)}
            onKeyDown={(event) => handleTreeKeyDown(event, null)}
            onClick={() => void onSelect(null, null)}
            ref={(element) => registerTreeItem(null, element)}
            role="treeitem"
            tabIndex={focusedRunId === null ? 0 : -1}
            type="button"
          >
            <CircleDot aria-hidden="true" size={14} />
            <span>
              <strong>工作区起点</strong>
              <small>虚拟根 · 不绑定历史 Run</small>
            </span>
            {projection.cursor.activeRunId === null ? <em>CURRENT CONTEXT</em> : null}
          </button>
        </div>

        {visibleNodes.map((node) => {
          const nodeBranches = node.branchIds
            .map((branchId) => branchById.get(branchId))
            .filter((branch): branch is ContextBranchView => Boolean(branch));
          const headBranch = nodeBranches.find((branch) => branch.headRunId === node.runId);
          const checkpoints = node.checkpointIds
            .map((checkpointId) => checkpointById.get(checkpointId))
            .filter((checkpoint): checkpoint is ContextCheckpointView => Boolean(checkpoint));
          const status = statusLabels[node.status] ?? node.status;

          return (
            <div
              className={`context-tree__row ${node.isOnActivePath ? "is-path" : "is-sibling"}`}
              key={node.runId}
              style={{ "--context-depth": depthByRun.get(node.runId) ?? 1 } as React.CSSProperties}
            >
              {checkpoints.map((checkpoint) => (
                <div className="context-tree__checkpoint" key={checkpoint.id}>
                  <span />
                  <strong>{checkpointLabel(checkpoint)}</strong>
                  <code>{checkpoint.sourceHash}</code>
                </div>
              ))}
              <button
                aria-current={node.isActive ? "true" : undefined}
                aria-label={`${node.title?.trim() || node.prompt} · ${node.runId} · ${status}${node.isActive ? " · 当前 Context" : ""}`}
                aria-level={(depthByRun.get(node.runId) ?? 1) + 1}
                className={node.isActive ? "is-active" : ""}
                disabled={busy}
                onFocus={() => setFocusedRunId(node.runId)}
                onKeyDown={(event) => handleTreeKeyDown(event, node.runId)}
                onClick={() =>
                  void onSelect(
                    node.runId,
                    resolveContextBranchId(projection, node.branchIds),
                  )}
                ref={(element) => registerTreeItem(node.runId, element)}
                role="treeitem"
                tabIndex={focusedRunId === node.runId ? 0 : -1}
                type="button"
              >
                <span className={`context-tree__status is-${node.status}`} aria-hidden="true" />
                <span className="context-tree__copy">
                  <strong>{node.title?.trim() || node.prompt}</strong>
                  <span>{node.outputPreview || "该运行没有输出"}</span>
                  <small>
                    <code>{node.runId}</code>
                    <span>{node.model}</span>
                    <span>{status}</span>
                    {!node.canContinue ? <b>不可继续</b> : null}
                  </small>
                </span>
                {node.isActive ? <em>CURRENT CONTEXT</em> : null}
              </button>

              {headBranch ? (
                <div className="context-tree__branch">
                  {editingBranchId === headBranch.id ? (
                    <form
                      aria-label={`重命名分支 ${headBranch.name}`}
                      onSubmit={(event) => submitRename(event, headBranch)}
                    >
                      <GitBranch aria-hidden="true" size={12} />
                      <input
                        aria-label="分支名称"
                        autoFocus
                        onChange={(event) => setBranchName(event.target.value)}
                        value={branchName}
                      />
                      <button aria-label="保存分支名称" type="submit">
                        <Check aria-hidden="true" size={12} />
                      </button>
                      <button
                        aria-label="取消重命名"
                        onClick={() => setEditingBranchId(null)}
                        type="button"
                      >
                        <X aria-hidden="true" size={12} />
                      </button>
                    </form>
                  ) : (
                    <>
                      <GitBranch aria-hidden="true" size={12} />
                      <span>{headBranch.name}</span>
                      {onRenameBranch ? (
                        <button
                          aria-label={`重命名分支 ${headBranch.name}`}
                          disabled={busy}
                          onClick={() => startRename(headBranch)}
                          type="button"
                        >
                          <Pencil aria-hidden="true" size={11} />
                        </button>
                      ) : null}
                    </>
                  )}
                </div>
              ) : null}
            </div>
          );
        })}

        {visibleNodes.length === 0 ? (
          <p className="context-tree__empty">没有匹配的 Context 节点。</p>
        ) : null}
      </div>
    </section>
  );
}
