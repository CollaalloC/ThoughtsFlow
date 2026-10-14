import { useCallback, useEffect, useState } from "react";

interface Page<T> { items: T[]; nextCursor: string | null }

// Opaque cursor navigation keeps one bounded summary page mounted at a time.
export function useAgentHistoryPage<T>(load: (cursor?: string) => Promise<Page<T>>, revision: string) {
  const [cursors, setCursors] = useState<Array<string | undefined>>([undefined]);
  const [pageNumber, setPageNumber] = useState(1);
  const [page, setPage] = useState<Page<T>>({ items: [], nextCursor: null });
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const cursor = cursors[pageNumber - 1];
  useEffect(() => {
    let active = true;
    setLoading(true);
    setPage({ items: [], nextCursor: null });
    setError(null);
    void load(cursor).then((result) => {
      if (active) setPage(result);
    }).catch((reason: unknown) => {
      if (active) setError(reason instanceof Error ? reason.message : "无法读取本地摘要。");
    }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [load, cursor, revision, refresh]);
  const previous = useCallback(() => { setPageNumber((current) => Math.max(1, current - 1)); }, []);
  const next = useCallback(() => {
    if (loading || !page.nextCursor) return;
    setCursors((current) => [...current.slice(0, pageNumber), page.nextCursor!]);
    setPageNumber(pageNumber + 1);
  }, [loading, page.nextCursor, pageNumber]);
  const latest = useCallback(() => {
    setCursors([undefined]);
    setPageNumber(1);
    setRefresh((current) => current + 1);
  }, []);
  return { ...page, pageNumber, loading, error, previous, next, latest };
}
