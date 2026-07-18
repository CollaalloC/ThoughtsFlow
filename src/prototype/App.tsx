/**
 * THROWAWAY UI PROTOTYPE
 * Three structural directions for the AI branching workspace, switchable via
 * `?variant=focus|canvas|trace` on the same route.
 */
import { useEffect, useState } from "react";
import { PrototypeSwitcher, variantMeta } from "./components/PrototypeSwitcher";
import { CanvasVariant } from "./variants/CanvasVariant";
import { FocusVariant } from "./variants/FocusVariant";
import { TraceVariant } from "./variants/TraceVariant";

export type VariantKey = "focus" | "canvas" | "trace";

function variantFromUrl(): VariantKey {
  const requested = new URLSearchParams(window.location.search).get("variant");
  return variantMeta.some((item) => item.key === requested)
    ? (requested as VariantKey)
    : "focus";
}

export function App() {
  const [variant, setVariantState] = useState<VariantKey>(variantFromUrl);

  const setVariant = (next: VariantKey) => {
    const url = new URL(window.location.href);
    url.searchParams.set("variant", next);
    window.history.replaceState({}, "", url);
    setVariantState(next);
  };

  useEffect(() => {
    const onPopState = () => setVariantState(variantFromUrl());
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (
        target?.matches("input, textarea, select, [contenteditable='true']") ||
        (event.key !== "ArrowLeft" && event.key !== "ArrowRight")
      ) {
        return;
      }
      const currentIndex = variantMeta.findIndex((item) => item.key === variant);
      const offset = event.key === "ArrowRight" ? 1 : -1;
      const next =
        variantMeta[
          (currentIndex + offset + variantMeta.length) % variantMeta.length
        ];
      setVariant(next.key);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [variant]);

  return (
    <main className={`prototype prototype--${variant}`}>
      {variant === "focus" && <FocusVariant />}
      {variant === "canvas" && <CanvasVariant />}
      {variant === "trace" && <TraceVariant />}
      <PrototypeSwitcher current={variant} onChange={setVariant} />
    </main>
  );
}
