import { ArrowLeft, ArrowRight, FlaskConical } from "lucide-react";
import type { VariantKey } from "../App";

export const variantMeta: Array<{ key: VariantKey; label: string }> = [
  { key: "focus", label: "专注路线" },
  { key: "canvas", label: "空间画布" },
  { key: "trace", label: "上下文凭证" },
];

type PrototypeSwitcherProps = {
  current: VariantKey;
  onChange: (variant: VariantKey) => void;
};

export function PrototypeSwitcher({ current, onChange }: PrototypeSwitcherProps) {
  if (!import.meta.env.DEV) return null;

  const index = variantMeta.findIndex((item) => item.key === current);
  const move = (offset: number) => {
    const next = variantMeta[(index + offset + variantMeta.length) % variantMeta.length];
    onChange(next.key);
  };

  return (
    <div className="prototype-switcher" aria-label="原型方向切换器">
      <span className="prototype-switcher__flag">
        <FlaskConical size={14} /> 原型
      </span>
      <button type="button" onClick={() => move(-1)} aria-label="上一个设计方向">
        <ArrowLeft size={16} />
      </button>
      <span className="prototype-switcher__label">
        {index + 1} / {variantMeta.length} · {variantMeta[index].label}
      </span>
      <button type="button" onClick={() => move(1)} aria-label="下一个设计方向">
        <ArrowRight size={16} />
      </button>
    </div>
  );
}
