import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
} from "react";
import type { DesktopBridge } from "../../platform/desktop-bridge";
import type { ModelRun, SessionCredentialSummary } from "../../shared/contracts";
import "./credential-recovery.css";

const RECOVERABLE_CREDENTIAL_ERRORS = new Set(["quota_exhausted", "rate_limited"]);

export interface CredentialRecoveryProps {
  bridge: DesktopBridge;
  run: ModelRun;
  disabled?: boolean;
  onRetryWithCredential(providerProfileId: string, credentialId: string): Promise<void>;
  onOpenSettings?(providerProfileId: string): void;
}

function orderedCredentials(credentials: SessionCredentialSummary[]) {
  return [...credentials].sort((left, right) => left.order - right.order);
}

function nextBackupCredential(credentials: SessionCredentialSummary[]) {
  const ordered = orderedCredentials(credentials);
  const active = ordered.find((credential) => credential.isActive);
  return (
    ordered.find(
      (credential) => !credential.isActive && (!active || credential.order > active.order),
    )
    ?? ordered.find((credential) => !credential.isActive)
    ?? null
  );
}

function errorMessage(reason: unknown, fallback: string) {
  return reason instanceof Error && reason.message ? reason.message : fallback;
}

export function CredentialRecovery({
  bridge,
  run,
  disabled = false,
  onRetryWithCredential,
  onOpenSettings,
}: CredentialRecoveryProps) {
  const panelId = useId();
  const errorRef = useRef<HTMLParagraphElement>(null);
  const inFlightRef = useRef(false);
  const loadSequenceRef = useRef(0);
  const [expanded, setExpanded] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [credentials, setCredentials] = useState<SessionCredentialSummary[]>([]);
  const [selectedCredentialId, setSelectedCredentialId] = useState("");
  const [switching, setSwitching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    loadSequenceRef.current += 1;
    inFlightRef.current = false;
    setExpanded(false);
    setLoaded(false);
    setLoading(false);
    setCredentials([]);
    setSelectedCredentialId("");
    setSwitching(false);
    setError(null);
    setStatus(null);
    return () => {
      loadSequenceRef.current += 1;
      inFlightRef.current = false;
    };
  }, [run.id, run.providerProfileId]);

  useEffect(() => {
    if (error) errorRef.current?.focus();
  }, [error]);

  const backups = useMemo(
    () => orderedCredentials(credentials).filter((credential) => !credential.isActive),
    [credentials],
  );

  const eligible = Boolean(
    run.error?.retryable
      && RECOVERABLE_CREDENTIAL_ERRORS.has(run.error.code),
  );

  if (!eligible) return null;

  const loadCredentials = async () => {
    const sequence = ++loadSequenceRef.current;
    setLoading(true);
    setError(null);
    setStatus(null);
    try {
      const summaries = await bridge.listSessionCredentials({
        providerProfileId: run.providerProfileId,
      });
      if (sequence !== loadSequenceRef.current) return;
      const ordered = orderedCredentials(summaries);
      setCredentials(ordered);
      setSelectedCredentialId(nextBackupCredential(ordered)?.credentialId ?? "");
      setLoaded(true);
    } catch (reason) {
      if (sequence !== loadSequenceRef.current) return;
      setError(errorMessage(reason, "无法读取本次会话的备用凭据。"));
    } finally {
      if (sequence === loadSequenceRef.current) setLoading(false);
    }
  };

  const toggleExpanded = () => {
    if (expanded) {
      setExpanded(false);
      return;
    }
    setExpanded(true);
    if (!loaded && !loading) void loadCredentials();
  };

  const changeSelection = (event: ChangeEvent<HTMLSelectElement>) => {
    setSelectedCredentialId(event.target.value);
    setError(null);
    setStatus(null);
  };

  const switchCredentialAndRetry = async () => {
    if (inFlightRef.current || disabled || !selectedCredentialId) return;
    const sequence = loadSequenceRef.current;
    inFlightRef.current = true;
    setSwitching(true);
    setError(null);
    setStatus(null);
    try {
      await onRetryWithCredential(run.providerProfileId, selectedCredentialId);
      if (sequence !== loadSequenceRef.current) return;
      setStatus("已用所选凭据请求新增回答版本，并将其设为当前首选。");
    } catch (reason) {
      if (sequence === loadSequenceRef.current) {
        setError(errorMessage(reason, "切换凭据或新增回答版本失败。"));
      }
    } finally {
      if (sequence === loadSequenceRef.current) {
        inFlightRef.current = false;
        setSwitching(false);
      }
    }
  };

  return (
    <section className="credential-recovery" role="region" aria-label="凭据恢复">
      <div className="credential-recovery__summary">
        <div>
          <strong>可更换会话凭据后重试</strong>
          <p>
            旧的失败回答和部分输出会原样保留；继续后会新增一个回答版本，不会覆盖历史。
          </p>
        </div>
        <button
          type="button"
          className="credential-recovery__toggle"
          aria-expanded={expanded}
          aria-controls={panelId}
          disabled={disabled || switching}
          onClick={toggleExpanded}
        >
          {expanded ? "收起凭据选择" : "选择备用凭据"}
        </button>
      </div>

      {expanded && (
        <div className="credential-recovery__panel" id={panelId}>
          {loading && <p role="status">正在读取安全凭据标签…</p>}

          {!loading && loaded && backups.length > 0 && (
            <>
              <label className="credential-recovery__field">
                <span>备用凭据</span>
                <select
                  aria-label="备用凭据"
                  value={selectedCredentialId}
                  disabled={disabled || switching}
                  onChange={changeSelection}
                >
                  {backups.map((credential) => (
                    <option key={credential.credentialId} value={credential.credentialId}>
                      {credential.label}
                    </option>
                  ))}
                </select>
              </label>
              <button
                type="button"
                className="credential-recovery__retry"
                disabled={disabled || switching || !selectedCredentialId}
                onClick={() => void switchCredentialAndRetry()}
              >
                {switching ? "正在切换…" : "切换并新增回答版本"}
              </button>
            </>
          )}

          {!loading && loaded && backups.length === 0 && (
            <div className="credential-recovery__empty">
              <p>当前会话没有可用的备用凭据。</p>
              {onOpenSettings && (
                <button
                  type="button"
                  disabled={disabled || switching}
                  onClick={() => onOpenSettings(run.providerProfileId)}
                >
                  打开此 Provider 设置
                </button>
              )}
            </div>
          )}

          {error && (
            <p
              ref={errorRef}
              className="credential-recovery__message is-error"
              role="alert"
              tabIndex={-1}
            >
              {error}
            </p>
          )}
          {status && (
            <p className="credential-recovery__message" role="status">
              {status}
            </p>
          )}
        </div>
      )}
    </section>
  );
}
