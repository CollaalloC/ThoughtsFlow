import { AlertCircle, HardDrive, LoaderCircle, Sparkles } from "lucide-react";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

export function Brand({ compact = false }: { compact?: boolean }) {
  return (
    <span className="tf-brand" aria-label="ThoughtsFlow">
      <span className="tf-brand__mark" aria-hidden="true">
        <i />
        <i />
        <i />
      </span>
      {!compact && (
        <span className="tf-brand__word">
          Thoughs<span>Flow</span>
        </span>
      )}
    </span>
  );
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  icon?: ReactNode;
  tone?: "default" | "primary" | "quiet" | "danger";
};

export function Button({
  children,
  className = "",
  icon,
  tone = "default",
  type = "button",
  ...props
}: ButtonProps) {
  return (
    <button
      type={type}
      className={`tf-button tf-button--${tone} ${className}`}
      {...props}
    >
      {icon}
      {children}
    </button>
  );
}

export function LocalDataBadge() {
  return (
    <span className="tf-local-badge">
      <HardDrive size={13} aria-hidden="true" /> 数据保存在本机
    </span>
  );
}

export function ProviderDestination({
  name,
  baseUrl,
  local,
  compact = false,
}: {
  name: string;
  baseUrl: string;
  local: boolean;
  compact?: boolean;
}) {
  let host = baseUrl;
  try {
    host = new URL(baseUrl).host;
  } catch {
    // Keep the user-provided value visible when it is still being configured.
  }

  return (
    <span className={`tf-destination ${local ? "is-local" : "is-remote"}`}>
      <span className="tf-destination__dot" aria-hidden="true" />
      {local ? "本机" : "外发"} · {compact ? host : `${name} · ${host}`}
    </span>
  );
}

export function LoadingState({ label = "正在读取本地工作区" }: { label?: string }) {
  return (
    <div className="tf-state" role="status">
      <LoaderCircle className="tf-spin" size={20} aria-hidden="true" />
      <span>{label}</span>
    </div>
  );
}

export function ErrorState({ message }: { message: string }) {
  return (
    <div className="tf-state tf-state--error" role="alert">
      <AlertCircle size={19} aria-hidden="true" />
      <span>{message}</span>
    </div>
  );
}

export function FlowMark() {
  return (
    <span className="tf-flow-mark" aria-hidden="true">
      <Sparkles size={14} />
    </span>
  );
}

export function SafeMarkdown({ children }: { children: string }) {
  return (
    <div className="tf-markdown">
      <ReactMarkdown
        skipHtml
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ children: linkChildren, ...props }) => (
            <a {...props} target="_blank" rel="noreferrer">{linkChildren}</a>
          ),
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}
