import { useEffect, useState } from "react";

/** Local ownership of UI continuations; it never cancels a persisted backend operation. */
export function useWorkspaceSession() {
  const [session] = useState(() => {
    let active = true;
    let workspaceId: string | undefined;
    let generation = 0;
    let navigation = 0;
    let revision = 0;
    const reads = new Map<string, number>();
    return {
      get generation() { return generation; },
      capture(expectedWorkspaceId: string | undefined) {
        const captured = generation;
        return () => active && generation === captured && workspaceId === expectedWorkspaceId;
      },
      navigate() {
        const captured = ++navigation;
        return () => active && navigation === captured;
      },
      read(expectedWorkspaceId: string | undefined, channel: "workspace" | "preview" | "send-preview" | "snapshot") {
        const ownsWorkspace = this.capture(expectedWorkspaceId);
        const captured = revision;
        const request = (reads.get(channel) ?? 0) + 1;
        reads.set(channel, request);
        return () => ownsWorkspace() && revision === captured && reads.get(channel) === request;
      },
      invalidateReads() { revision += 1; },
      activate(nextWorkspaceId: string | undefined) {
        if (workspaceId === nextWorkspaceId) return false;
        workspaceId = nextWorkspaceId;
        generation += 1;
        return true;
      },
      mount() { active = true; },
      dispose() { active = false; generation += 1; navigation += 1; },
    };
  });
  useEffect(() => {
    session.mount();
    return () => session.dispose();
  }, [session]);
  return session;
}
