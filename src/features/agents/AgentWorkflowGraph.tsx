import { memo, useMemo } from "react";
import { Background, Controls, MarkerType, ReactFlow, type Edge, type Node } from "@xyflow/react";
import type { AgentEvidenceSummary, AgentHandoffSummary } from "../../shared/contracts";
import "@xyflow/react/dist/style.css";

export const AgentWorkflowGraph = memo(function AgentWorkflowGraph({ handoffs, evidence, runTitles }: { handoffs: AgentHandoffSummary[]; evidence: AgentEvidenceSummary[]; runTitles: Record<string, string> }) {
  const graph = useMemo(() => {
    const titles = new Map(handoffs.map((item) => [item.operationId, item.title?.trim()]));
    const handoffTitle = (id: string) => titles.get(id) || `任务交接 ${id.slice(0, 8)}`;
    const nodes: Node[] = [];
    const edges: Edge[] = [];
    const ids = new Set<string>();
    const lanes = [0, 0, 0];
    const addNode = (id: string, label: string, lane: number) => {
      if (ids.has(id)) return;
      ids.add(id);
      nodes.push({ id, position: { x: lane * 260, y: lanes[lane]++ * 100 }, data: { label } });
    };
    const link = (source: string, target: string) => edges.push({ id: `${source}->${target}`, source, target, markerEnd: { type: MarkerType.ArrowClosed } });
    evidence.forEach((item) => {
      const source = item.handoffOperationId ? `handoff:${item.handoffOperationId}` : `agent-run:${item.runId}`;
      addNode(source, item.handoffOperationId ? handoffTitle(item.handoffOperationId) : `Agent 协作 ${item.runId.slice(0, 8)}`, 1);
      addNode(`evidence:${item.id}`, `输出观察 · ${item.title?.trim() || (item.handoffOperationId ? titles.get(item.handoffOperationId) : null) || item.taskId.slice(0, 12)}\n${item.hasMore ? "有后续页" : "一页快照"}`, 2);
      link(source, `evidence:${item.id}`);
    });
    handoffs.forEach((item) => {
      const target = `handoff:${item.operationId}`;
      addNode(target, handoffTitle(item.operationId), 1);
      item.sourceRunIds.forEach((id) => {
        addNode(`context:${id}`, `上下文回答 · ${runTitles[id] || id.slice(0, 12)}`, 0);
        link(`context:${id}`, target);
      });
      item.evidenceIds.forEach((id) => {
        addNode(`evidence:${id}`, `输出观察 ${id.slice(0, 12)}`, 2);
        link(`evidence:${id}`, target);
      });
    });
    return { nodes, edges };
  }, [handoffs, evidence, runTitles]);
  if (!graph.nodes.length) return null;
  return <section aria-label="人工编排来源图" className="agent-workspace__panel">
    <h4>已加载记录来源图</h4>
    <p>只显示当前摘要页及其直接来源，其他历史需翻页查看。箭头表示你选择的输入来源；每次任务由你预览后启动，不会自动调度或重派。</p>
    <div className="agent-workspace__graph">
      <ReactFlow nodes={graph.nodes} edges={graph.edges} fitView nodesDraggable={false} nodesConnectable={false} elementsSelectable={false} minZoom={0.2} maxZoom={1.5}>
        <Background /><Controls showInteractive={false} />
      </ReactFlow>
    </div>
  </section>;
});
