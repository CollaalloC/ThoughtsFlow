import {
  Check,
  ChevronDown,
  Cloud,
  HardDrive,
  MoreHorizontal,
} from "lucide-react";
import type { ReactNode } from "react";

export function Brand({ compact = false }: { compact?: boolean }) {
  return (
    <div className={`brand ${compact ? "brand--compact" : ""}`}>
      <span className="brand__mark" aria-hidden="true">
        <span />
        <span />
        <span />
      </span>
      {!compact && (
        <span className="brand__text">
          Thoughs<span>Flow</span>
        </span>
      )}
    </div>
  );
}

export function LocalBadge({ compact = false }: { compact?: boolean }) {
  return (
    <span className="local-badge" title="工作区数据保存在本机">
      <HardDrive size={13} />
      {!compact && "数据在本机"}
    </span>
  );
}

export function EndpointBadge({ local = false }: { local?: boolean }) {
  return (
    <span className={`endpoint-badge ${local ? "endpoint-badge--local" : ""}`}>
      {local ? <HardDrive size={12} /> : <Cloud size={12} />}
      {local ? "本机 Ollama" : "外发 · OpenAI"}
    </span>
  );
}

export function StatusDot({ status }: { status: string }) {
  return <span className={`status-dot status-dot--${status}`} aria-label={status} />;
}

type ButtonProps = {
  children: ReactNode;
  icon?: ReactNode;
  className?: string;
  onClick?: () => void;
  active?: boolean;
  disabled?: boolean;
  title?: string;
};

export function Button({
  children,
  icon,
  className = "",
  onClick,
  active = false,
  disabled = false,
  title,
}: ButtonProps) {
  return (
    <button
      type="button"
      className={`ui-button ${active ? "is-active" : ""} ${className}`}
      onClick={onClick}
      disabled={disabled}
      title={title}
    >
      {icon}
      {children}
    </button>
  );
}

export function MenuButton({ label = "更多" }: { label?: string }) {
  return (
    <button type="button" className="icon-button" aria-label={label} title={label}>
      <MoreHorizontal size={17} />
    </button>
  );
}

export function ModelSelect({ local = false }: { local?: boolean }) {
  return (
    <button type="button" className="model-select">
      <span className={`model-select__dot ${local ? "is-local" : ""}`} />
      <span>{local ? "Qwen3:14b" : "GPT-4.1"}</span>
      <ChevronDown size={13} />
    </button>
  );
}

export function SavedState() {
  return (
    <span className="saved-state">
      <Check size={12} /> 刚刚已保存
    </span>
  );
}
