export type RunStatus =
  | "completed"
  | "streaming"
  | "interrupted"
  | "failed";

export type ModelRun = {
  id: string;
  label: string;
  model: string;
  provider: string;
  status: RunStatus;
  duration: string;
  output: string;
  excerpt: string;
  usage?: string;
};

export type Turn = {
  id: string;
  parentId: string | null;
  parentRunId: string | null;
  title: string;
  prompt: string;
  branchLabel: string;
  branchCount: number;
  createdAt: string;
  runs: ModelRun[];
  selectedRunId: string;
  x: number;
  y: number;
  tone: "root" | "main" | "branch" | "muted";
};

export type ContextItem = {
  id: string;
  ordinal: number;
  kind: "system" | "prompt" | "answer" | "pinned" | "current";
  label: string;
  source: string;
  preview: string;
  tokens: number;
  included: boolean;
  reason: string;
};

export const workspace = {
  name: "AI 分支对话产品定义",
  goal: "确定首版默认导航、上下文透明度与长期使用价值",
  storage: "本机",
  lastOpened: "今天 21:48",
  saveState: "刚刚已保存",
};

export const workspaces = [
  { name: "AI 分支对话产品定义", count: 18, active: true },
  { name: "Tauri 架构评估", count: 11, active: false },
  { name: "用户访谈提纲", count: 7, active: false },
];

export const turns: Turn[] = [
  {
    id: "root",
    parentId: null,
    parentRunId: null,
    title: "真正要解决的问题",
    prompt: "分支对话产品真正要解决的核心问题是什么？",
    branchLabel: "根问题",
    branchCount: 3,
    createdAt: "7 月 16 日 09:42",
    selectedRunId: "root-a",
    x: 72,
    y: 252,
    tone: "root",
    runs: [
      {
        id: "root-a",
        label: "回答 A",
        model: "GPT-4.1",
        provider: "OpenAI",
        status: "completed",
        duration: "42s",
        usage: "2.1k → 842",
        excerpt: "核心不是把聊天画成脑图，而是让上下文路径可见、可控、可追溯。",
        output:
          "核心问题不是“如何把聊天画成脑图”，而是如何让复杂任务中的每次推演都拥有明确的上下文边界。用户需要从任意回答继续另一条路线，同时确信旁支不会污染主线，并能在发送前确认模型此刻真正会看到什么。",
      },
    ],
  },
  {
    id: "focus",
    parentId: "root",
    parentRunId: "root-a",
    title: "默认界面应该是什么",
    prompt: "默认界面应该是线性阅读，还是无限画布？",
    branchLabel: "默认导航",
    branchCount: 3,
    createdAt: "7 月 16 日 10:18",
    selectedRunId: "focus-b",
    x: 356,
    y: 118,
    tone: "main",
    runs: [
      {
        id: "focus-a",
        label: "回答 A",
        model: "GPT-4.1",
        provider: "OpenAI",
        status: "completed",
        duration: "38s",
        usage: "3.3k → 1.1k",
        excerpt: "默认线性阅读，画布只在需要鸟瞰、折叠和跨分支定位时出现。",
        output:
          "建议默认采用线性阅读。用户首先需要读懂当前路线，而不是理解整张图。画布应当作为可切换的路线图：负责鸟瞰、折叠和定位，不负责承载完整回答，也不成为上下文语义的真相。",
      },
      {
        id: "focus-b",
        label: "回答 B",
        model: "Qwen3:14b",
        provider: "Ollama · 本机",
        status: "completed",
        duration: "1m 24s",
        usage: "3.3k → 986",
        excerpt: "以专注阅读为主场，以路线图维持空间记忆，用同一选中状态连接两者。",
        output:
          "更稳妥的结构是“专注阅读 + 路线图”双视图。专注模式保留熟悉的对话节奏；路线图提供空间记忆。两种视图共享同一个当前 Turn 与回答版本，因此切换视图不会丢失位置。",
      },
    ],
  },
  {
    id: "canvas",
    parentId: "focus",
    parentRunId: "focus-b",
    title: "画布如何避免迷路",
    prompt: "当节点超过 30 个时，画布怎样避免成为新的迷宫？",
    branchLabel: "画布优先",
    branchCount: 1,
    createdAt: "7 月 17 日 11:08",
    selectedRunId: "canvas-a",
    x: 648,
    y: 34,
    tone: "branch",
    runs: [
      {
        id: "canvas-a",
        label: "回答 A",
        model: "GPT-4.1",
        provider: "OpenAI",
        status: "completed",
        duration: "51s",
        usage: "4.8k → 1.4k",
        excerpt: "强化当前路线、折叠旁支，并用语义地标替代无限缩放找节点。",
        output:
          "大树导航不应只依赖缩放。默认强化当前祖先路径，兄弟分支降权；用结论、待验证问题和最近位置作为语义地标。子树可折叠，搜索结果直接定位并保留返回路径。",
      },
    ],
  },
  {
    id: "context",
    parentId: "focus",
    parentRunId: "focus-b",
    title: "Context Inspector 渐进披露",
    prompt: "怎样让 Context Inspector 有价值，但不增加每轮对话的负担？",
    branchLabel: "上下文透明",
    branchCount: 2,
    createdAt: "7 月 17 日 21:48",
    selectedRunId: "context-a",
    x: 648,
    y: 230,
    tone: "main",
    runs: [
      {
        id: "context-a",
        label: "回答 A",
        model: "GPT-4.1",
        provider: "OpenAI",
        status: "completed",
        duration: "46s",
        usage: "5.6k → 1.2k",
        excerpt: "默认只显示一行发送凭证；有疑问时再展开有序的上下文清单。",
        output:
          "把 Inspector 设计成渐进披露的“发送凭证”。默认只显示祖先项数、固定/排除数量、估算大小和发送目标；用户发现回答异常、切换端点或上下文超限时，再展开有序清单。请求发出后，该清单锁定为可检查的历史快照。",
      },
      {
        id: "context-b",
        label: "回答 B",
        model: "Qwen3:14b",
        provider: "Ollama · 本机",
        status: "interrupted",
        duration: "18s",
        excerpt: "上次运行被中断，已恢复 643 字；接受后才可以继续分支。",
        usage: "5.6k → 643 字",
        output:
          "Context Inspector 可以分为三层：Composer 上方的摘要凭证、可调整的下一次发送清单，以及历史运行的只读快照。对于本机模型，端点同样应保持可见……",
      },
    ],
  },
  {
    id: "local",
    parentId: "focus",
    parentRunId: "focus-a",
    title: "如何表达本地优先",
    prompt: "怎样准确表达本地优先，而不让用户误以为完全离线？",
    branchLabel: "本地优先",
    branchCount: 0,
    createdAt: "7 月 17 日 15:30",
    selectedRunId: "local-a",
    x: 648,
    y: 430,
    tone: "branch",
    runs: [
      {
        id: "local-a",
        label: "回答 A",
        model: "GPT-4.1",
        provider: "OpenAI",
        status: "completed",
        duration: "29s",
        usage: "4.1k → 768",
        excerpt: "明确区分“工作区保存在本机”与“本轮内容将发送到哪个模型端点”。",
        output:
          "“数据保存在本机”和“推理请求是否外发”必须分开表达。状态栏说明工作区存储位置；发送凭证明确列出 provider、model 与 base URL。切换到云端时再进行一次边界确认。",
      },
    ],
  },
  {
    id: "agent",
    parentId: "root",
    parentRunId: "root-a",
    title: "Agent 工作台",
    prompt: "首版是否应该加入 Agent 与 MCP？",
    branchLabel: "已搁置",
    branchCount: 0,
    createdAt: "7 月 16 日 14:02",
    selectedRunId: "agent-a",
    x: 356,
    y: 520,
    tone: "muted",
    runs: [
      {
        id: "agent-a",
        label: "回答 A",
        model: "GPT-4.1",
        provider: "OpenAI",
        status: "completed",
        duration: "33s",
        excerpt: "后置。它会掩盖真正需要验证的分支与上下文价值。",
        output:
          "Agent 与 MCP 同时引入审批、权限、沙箱、长任务恢复和审计。首版加入它们，会掩盖真正需要验证的问题：用户是否会持续使用分支和上下文控制。",
      },
    ],
  },
];

export const selectedTurnId = "context";

export const contextItems: ContextItem[] = [
  {
    id: "ctx-system",
    ordinal: 1,
    kind: "system",
    label: "系统说明",
    source: "工作区默认",
    preview: "你是一名严谨的 AI 产品设计顾问……",
    tokens: 186,
    included: true,
    reason: "工作区系统说明",
  },
  {
    id: "ctx-root-prompt",
    ordinal: 2,
    kind: "prompt",
    label: "根问题",
    source: "真正要解决的问题",
    preview: "分支对话产品真正要解决的核心问题是什么？",
    tokens: 42,
    included: true,
    reason: "位于当前祖先路线",
  },
  {
    id: "ctx-root-answer",
    ordinal: 3,
    kind: "answer",
    label: "回答 A · GPT-4.1",
    source: "真正要解决的问题",
    preview: "核心不是把聊天画成脑图，而是让上下文路径可见、可控……",
    tokens: 842,
    included: true,
    reason: "子分支绑定到此回答版本",
  },
  {
    id: "ctx-focus-prompt",
    ordinal: 4,
    kind: "prompt",
    label: "默认导航",
    source: "默认界面应该是什么",
    preview: "默认界面应该是线性阅读，还是无限画布？",
    tokens: 38,
    included: true,
    reason: "位于当前祖先路线",
  },
  {
    id: "ctx-focus-answer",
    ordinal: 5,
    kind: "answer",
    label: "回答 B · Qwen3:14b",
    source: "默认界面应该是什么",
    preview: "以专注阅读为主场，以路线图维持空间记忆……",
    tokens: 986,
    included: true,
    reason: "当前 Turn 精确绑定的父回答",
  },
  {
    id: "ctx-pin",
    ordinal: 6,
    kind: "pinned",
    label: "已固定结论",
    source: "用户访谈计划 · 回答 A",
    preview: "跨天恢复依赖位置、结论和下一步，而不是完整鸟瞰。",
    tokens: 124,
    included: true,
    reason: "你手动固定了这条结论",
  },
  {
    id: "ctx-current",
    ordinal: 7,
    kind: "current",
    label: "当前问题",
    source: "新问题",
    preview: "怎样让 Context Inspector 有价值，但不增加每轮对话的负担？",
    tokens: 46,
    included: true,
    reason: "本轮输入",
  },
  {
    id: "ctx-excluded",
    ordinal: 8,
    kind: "answer",
    label: "回答 A · 早期假设",
    source: "画布优先探索",
    preview: "所有操作都应该直接发生在无限画布上。",
    tokens: 360,
    included: false,
    reason: "你为下一次发送排除了此项",
  },
];

export const conclusions = [
  "画布只是导航，不是上下文语义真相",
  "一个问题可以保留多个回答版本",
  "跨天恢复需要位置、结论和下一步",
  "分支合并应表达为显式引用",
];

export const openQuestions = [
  "30+ Turn 后，路线图是否仍优于线性列表？",
  "用户会在错误发生前主动检查上下文吗？",
  "回答版本是否会被误认为独立分支？",
];

export function getTurn(id: string) {
  return turns.find((turn) => turn.id === id) ?? turns[0];
}

export function getSelectedRun(turn: Turn) {
  return turn.runs.find((run) => run.id === turn.selectedRunId) ?? turn.runs[0];
}
