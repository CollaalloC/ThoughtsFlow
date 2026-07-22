import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import type { DesktopBridge } from "../../platform/desktop-bridge";
import type { SessionCredentialSummary } from "../../shared/contracts";
import "./session-credential-manager.css";

export interface SessionCredentialManagerProps {
  bridge: DesktopBridge;
  providerProfileId: string;
  disabled?: boolean;
}

function errorMessage(reason: unknown, fallback: string) {
  return reason instanceof Error ? reason.message : fallback;
}

export function SessionCredentialManager({
  bridge,
  providerProfileId,
  disabled = false,
}: SessionCredentialManagerProps) {
  const [credentials, setCredentials] = useState<SessionCredentialSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [label, setLabel] = useState("");
  const [saving, setSaving] = useState(false);
  const [managing, setManaging] = useState(false);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const passwordRef = useRef<HTMLInputElement>(null);
  const labelRef = useRef<HTMLInputElement>(null);
  const errorRef = useRef<HTMLParagraphElement>(null);
  const deleteReturnFocusRef = useRef<{
    providerProfileId: string;
    credentialId: string;
    element: HTMLButtonElement;
  } | null>(null);
  const currentProviderProfileIdRef = useRef(providerProfileId);
  const requestGeneration = useRef(0);
  const headingId = useId();
  const orderedCredentials = [...credentials].sort((left, right) => left.order - right.order);
  currentProviderProfileIdRef.current = providerProfileId;

  useEffect(() => {
    if (error) errorRef.current?.focus();
  }, [error]);

  useEffect(() => {
    const generation = ++requestGeneration.current;
    let active = true;
    setLoading(true);
    setSaving(false);
    setManaging(false);
    setError(null);
    setStatus(null);
    setCredentials([]);
    setPendingDeleteId(null);
    deleteReturnFocusRef.current = null;
    setLabel("");
    if (passwordRef.current) passwordRef.current.value = "";
    bridge.listSessionCredentials({ providerProfileId })
      .then((summaries) => {
        if (active && generation === requestGeneration.current) setCredentials(summaries);
      })
      .catch((reason: unknown) => {
        if (!active || generation !== requestGeneration.current) return;
        setError(errorMessage(reason, "无法读取本次会话的凭据。"));
      })
      .finally(() => {
        if (active && generation === requestGeneration.current) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [bridge, providerProfileId]);

  const addCredential = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const credentialLabel = label.trim();
    const passwordInput = passwordRef.current;
    const credential = passwordInput?.value ?? "";
    if (passwordInput) passwordInput.value = "";
    setError(null);
    setStatus(null);

    if (!credentialLabel) {
      setError("请填写凭据标签。");
      return;
    }
    if (!credential) {
      setError("请输入 API Key。");
      return;
    }

    const generation = requestGeneration.current;
    setSaving(true);
    try {
      const summaries = await bridge.setSessionCredential({
        providerProfileId,
        credentialLabel,
        credential,
      });
      if (generation !== requestGeneration.current) return;
      setCredentials(summaries);
      setLabel("");
      setStatus(`已添加“${credentialLabel}”；凭据仅用于本次应用会话。`);
    } catch (reason) {
      if (generation === requestGeneration.current) {
        setError(errorMessage(reason, "添加会话凭据失败。"));
      }
    } finally {
      if (generation === requestGeneration.current) setSaving(false);
    }
  };

  const activateCredential = async (credential: SessionCredentialSummary) => {
    setError(null);
    setStatus(null);
    setManaging(true);
    const generation = requestGeneration.current;
    try {
      const summaries = await bridge.activateSessionCredential({
        providerProfileId,
        credentialId: credential.credentialId,
      });
      if (generation !== requestGeneration.current) return;
      setCredentials(summaries);
      setStatus(`“${credential.label}”已设为当前首选。`);
    } catch (reason) {
      if (generation === requestGeneration.current) {
        setError(errorMessage(reason, "切换首选凭据失败。"));
      }
    } finally {
      if (generation === requestGeneration.current) setManaging(false);
    }
  };

  const moveCredential = async (credentialId: string, offset: -1 | 1) => {
    const index = orderedCredentials.findIndex(
      (credential) => credential.credentialId === credentialId,
    );
    const destination = index + offset;
    if (index < 0 || destination < 0 || destination >= orderedCredentials.length) return;
    if (orderedCredentials[index]?.isActive || orderedCredentials[destination]?.isActive) return;
    const reordered = [...orderedCredentials];
    [reordered[index], reordered[destination]] = [reordered[destination], reordered[index]];

    setError(null);
    setStatus(null);
    setManaging(true);
    const generation = requestGeneration.current;
    try {
      const summaries = await bridge.reorderSessionCredentials({
        providerProfileId,
        orderedCredentialIds: reordered.map(({ credentialId }) => credentialId),
      });
      if (generation !== requestGeneration.current) return;
      setCredentials(summaries);
      setStatus("备用顺序已更新。");
    } catch (reason) {
      if (generation === requestGeneration.current) {
        setError(errorMessage(reason, "调整凭据顺序失败。"));
      }
    } finally {
      if (generation === requestGeneration.current) setManaging(false);
    }
  };

  const removeCredential = async (credential: SessionCredentialSummary) => {
    setError(null);
    setStatus(null);
    setManaging(true);
    const generation = requestGeneration.current;
    try {
      const summaries = await bridge.removeSessionCredential({
        providerProfileId,
        credentialId: credential.credentialId,
      });
      if (generation !== requestGeneration.current) return;
      setCredentials(summaries);
      setPendingDeleteId(null);
      deleteReturnFocusRef.current = null;
      setStatus(`已从本次会话移除“${credential.label}”。`);
      requestAnimationFrame(() => labelRef.current?.focus());
    } catch (reason) {
      if (generation === requestGeneration.current) {
        setError(errorMessage(reason, "移除会话凭据失败。"));
      }
    } finally {
      if (generation === requestGeneration.current) setManaging(false);
    }
  };

  const cancelDelete = () => {
    const returnFocus = deleteReturnFocusRef.current;
    const generation = requestGeneration.current;
    setPendingDeleteId(null);
    deleteReturnFocusRef.current = null;
    if (!returnFocus) return;
    requestAnimationFrame(() => {
      if (
        generation !== requestGeneration.current
        || currentProviderProfileIdRef.current !== returnFocus.providerProfileId
        || !returnFocus.element.isConnected
        || returnFocus.element.disabled
      ) return;
      returnFocus.element.focus();
    });
  };

  return (
    <section className="session-credentials" aria-labelledby={headingId}>
      <header className="session-credentials__header">
        <h3 id={headingId}>会话凭据</h3>
        <p>凭据只保存在 Rust 进程内存中，退出应用后会全部清除。</p>
      </header>
      {loading && (
        <p className="session-credentials__state" role="status">
          正在读取本次会话的凭据…
        </p>
      )}
      {!loading && credentials.length === 0 && !error && (
        <p className="session-credentials__state">本次会话还没有凭据。</p>
      )}
      {orderedCredentials.length > 0 && (
        <ul className="session-credentials__list" aria-label="本次会话凭据">
          {orderedCredentials.map((credential, index) => (
            <li
              className="session-credentials__item"
              key={credential.credentialId}
              aria-label={credential.label}
            >
              <div className="session-credentials__identity">
                <strong>{credential.label}</strong>
                <span className={credential.isActive ? "is-active" : ""}>
                  {credential.isActive
                    ? "当前首选"
                    : `备用 ${orderedCredentials
                      .slice(0, index)
                      .filter((candidate) => !candidate.isActive).length + 1}`}
                </span>
              </div>
              <div className="session-credentials__actions">
                {!credential.isActive && (
                  <button
                    type="button"
                    disabled={disabled || saving || managing}
                    onClick={() => activateCredential(credential)}
                  >
                    设为首选
                  </button>
                )}
                <button
                  type="button"
                  aria-label={`上移 ${credential.label}`}
                  disabled={
                    disabled
                    || saving
                    || managing
                    || credential.isActive
                    || index <= 1
                  }
                  onClick={() => moveCredential(credential.credentialId, -1)}
                >
                  上移
                </button>
                <button
                  type="button"
                  aria-label={`下移 ${credential.label}`}
                  disabled={
                    disabled
                    || saving
                    || managing
                    || credential.isActive
                    || index === orderedCredentials.length - 1
                  }
                  onClick={() => moveCredential(credential.credentialId, 1)}
                >
                  下移
                </button>
                <button
                  type="button"
                  aria-label={`删除 ${credential.label}`}
                  title={
                    credential.isActive && orderedCredentials.length > 1
                      ? "请先将另一凭据设为首选，再删除当前首选。"
                      : undefined
                  }
                  disabled={
                    disabled
                    || saving
                    || managing
                    || (credential.isActive && orderedCredentials.length > 1)
                  }
                  onClick={(event) => {
                    deleteReturnFocusRef.current = {
                      providerProfileId,
                      credentialId: credential.credentialId,
                      element: event.currentTarget,
                    };
                    setPendingDeleteId(credential.credentialId);
                    setError(null);
                    setStatus(null);
                  }}
                >
                  删除
                </button>
              </div>
              {pendingDeleteId === credential.credentialId && (
                <div
                  className="session-credentials__confirm"
                  role="group"
                  aria-label={`删除 ${credential.label}`}
                >
                  <p>确定从本次会话移除“{credential.label}”吗？</p>
                  <button
                    type="button"
                    aria-label={`确认删除 ${credential.label}`}
                    autoFocus
                    disabled={disabled || saving || managing}
                    onClick={() => removeCredential(credential)}
                  >
                    确认删除
                  </button>
                  <button
                    type="button"
                    disabled={disabled || saving || managing}
                    onClick={cancelDelete}
                  >
                    取消
                  </button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
      {error && (
        <p
          className="session-credentials__message is-error"
          ref={errorRef}
          role="alert"
          tabIndex={-1}
        >
          {error}
        </p>
      )}
      {status && (
        <p className="session-credentials__message is-success" role="status">
          {status}
        </p>
      )}
      <form className="session-credentials__form" onSubmit={addCredential}>
        <fieldset disabled={disabled || saving || managing || loading}>
          <legend>添加会话凭据</legend>
          <label>
            <span>凭据标签</span>
            <input
              ref={labelRef}
              aria-label="凭据标签"
              value={label}
              onChange={(event) => setLabel(event.target.value)}
              autoComplete="off"
            />
          </label>
          <label>
            <span>API Key（仅本次会话）</span>
            <input
              ref={passwordRef}
              aria-label="API Key（仅本次会话）"
              type="password"
              autoComplete="new-password"
            />
          </label>
          <button type="submit">{saving ? "正在添加…" : "添加会话凭据"}</button>
        </fieldset>
      </form>
    </section>
  );
}
