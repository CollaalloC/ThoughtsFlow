export interface AgentTaskDraft {
  title: string;
  spec: string;
  includeActiveContext: boolean;
  evidenceIds: string[];
}

/** Application-owned draft state; subscriptions reconcile a late confirmed dispatch. */
export class AgentTaskDrafts {
  private readonly drafts: Map<string, AgentTaskDraft>;
  private readonly listeners = new Set<() => void>();

  constructor(entries?: Iterable<readonly [string, AgentTaskDraft]>) {
    this.drafts = new Map(entries);
  }

  get(id: string) { return this.drafts.get(id); }
  has(id: string) { return this.drafts.has(id); }
  set(id: string, draft: AgentTaskDraft) {
    if (this.drafts.get(id) === draft) return this;
    this.drafts.set(id, draft);
    this.listeners.forEach((listener) => listener());
    return this;
  }
  delete(id: string) {
    const deleted = this.drafts.delete(id);
    if (deleted) this.listeners.forEach((listener) => listener());
    return deleted;
  }
  subscribe(listener: () => void) {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }
}
