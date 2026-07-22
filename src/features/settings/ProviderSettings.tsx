import { Check, Cloud, HardDrive, KeyRound, LoaderCircle, PlugZap, Plus } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import type { DesktopBridge } from "../../platform/desktop-bridge";
import type { ProviderProfile, ProviderTemplate } from "../../shared/contracts";
import { Button, LocalDataBadge, ProviderDestination } from "../../shared/ui";
import "./provider-settings.css";

type ProviderProfileView = ProviderProfile;

type Draft = {
  id?: string;
  providerId: string;
  name: string;
  baseUrl: string;
  defaultModel: string;
  credential: string;
};

const emptyDraft: Draft = {
  providerId: "",
  name: "",
  baseUrl: "",
  defaultModel: "",
  credential: "",
};

function isLoopbackUrl(url: URL) {
  return ["127.0.0.1", "localhost", "[::1]", "::1"].includes(url.hostname);
}

function isLocalEndpoint(rawUrl: string) {
  try {
    return isLoopbackUrl(new URL(rawUrl));
  } catch {
    return false;
  }
}

function validateEndpoint(rawUrl: string) {
  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    return "请输入完整的 Base URL，例如 https://api.example.com/v1。";
  }
  if (url.username || url.password) return "Base URL 不能包含用户名、密码或 API Key。";
  if (url.search || url.hash) return "Base URL 不能包含查询参数或片段。";
  if (url.protocol === "http:" && !isLoopbackUrl(url)) return "远程端点必须使用 HTTPS。";
  if (url.protocol !== "https:" && url.protocol !== "http:") return "Provider 端点只支持 HTTP 或 HTTPS。";
  return null;
}

function normalizeBaseUrl(rawUrl: string) {
  return rawUrl.trim().replace(/\/+$/, "");
}

function describeAuth(template?: ProviderTemplate) {
  if (!template || template.protocol.authPlacement === "none") return "无认证";

  const headerName = template.protocol.authHeaderName;
  if (template.protocol.authPlacement === "bearer_header") {
    return `${headerName ?? "Authorization"}: Bearer …`;
  }
  if (template.protocol.authPlacement === "api_key_header") {
    return `${headerName ?? "x-api-key"}: …`;
  }
  return `${headerName ?? "key"}=…（URL 查询参数）`;
}

export function ProviderSettings({ bridge }: { bridge: DesktopBridge }) {
  const [profiles, setProfiles] = useState<ProviderProfileView[]>([]);
  const [templates, setTemplates] = useState<ProviderTemplate[]>([]);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    Promise.all([bridge.listProviderProfiles(), bridge.listProviderTemplates()])
      .then(([nextProfiles, nextTemplates]) => {
        if (!active) return;
        setProfiles(nextProfiles as ProviderProfileView[]);
        setTemplates(nextTemplates);
        const firstAvailable = nextTemplates.find((template) => template.runtimeAvailable);
        if (firstAvailable) {
          setDraft((current) =>
            current.providerId
              ? current
              : {
                  ...current,
                  providerId: firstAvailable.providerId,
                  baseUrl: current.baseUrl || firstAvailable.defaultBaseUrl,
                },
          );
        }
      })
      .catch((reason: unknown) => {
        if (active) setError(reason instanceof Error ? reason.message : "无法读取 Provider 配置。");
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => { active = false; };
  }, [bridge]);

  const selectedTemplate = useMemo(
    () => templates.find((template) => template.providerId === draft.providerId),
    [draft.providerId, templates],
  );

  const connectionTestNeedsSave = useMemo(() => {
    if (!draft.id) return true;
    const persisted = profiles.find((profile) => profile.id === draft.id);
    if (!persisted) return true;
    return (
      draft.providerId !== persisted.providerId
      || draft.name.trim() !== persisted.name
      || normalizeBaseUrl(draft.baseUrl) !== normalizeBaseUrl(persisted.baseUrl)
      || draft.defaultModel.trim() !== persisted.model
      || draft.credential.length > 0
    );
  }, [draft, profiles]);

  const parsedHost = useMemo(() => {
    try {
      return new URL(draft.baseUrl).host;
    } catch {
      return "尚未配置的端点";
    }
  }, [draft.baseUrl]);

  const editProfile = (profile: ProviderProfileView) => {
    setDraft({
      id: profile.id,
      providerId: profile.providerId,
      name: profile.name,
      baseUrl: profile.baseUrl,
      defaultModel: profile.model,
      credential: "",
    });
    setError(null);
    setStatus(null);
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setError(null);
    setStatus(null);
    const endpointError = validateEndpoint(draft.baseUrl);
    if (endpointError) {
      setError(endpointError);
      return;
    }
    if (!draft.name.trim()) {
      setError("请填写 Provider 名称。");
      return;
    }
    if (!selectedTemplate) {
      setError("请选择 Provider 模板。");
      return;
    }
    if (!selectedTemplate.runtimeAvailable) {
      setError("该 Provider 协议尚未开放运行。");
      return;
    }
    if (!draft.defaultModel.trim()) {
      setError("请填写默认模型。");
      return;
    }

    setSaving(true);
    try {
      const wasExisting = Boolean(draft.id);
      const submittedCredential = draft.credential;
      const profileInput = {
        id: draft.id,
        providerId: draft.providerId,
        name: draft.name.trim(),
        baseUrl: normalizeBaseUrl(draft.baseUrl),
        model: draft.defaultModel.trim(),
        isDefault: profiles.length === 0,
        parameters: {},
      };
      const saved = (await (
        submittedCredential && selectedTemplate.protocol.authPlacement !== "none"
          ? bridge.saveProviderProfile(profileInput, submittedCredential)
          : bridge.saveProviderProfile(profileInput)
      )) as ProviderProfileView;
      setProfiles((current) => {
        const withoutSaved = current.filter((profile) => profile.id !== saved.id);
        return [...withoutSaved, saved];
      });
      setDraft({
        id: saved.id,
        providerId: saved.providerId,
        name: saved.name,
        baseUrl: saved.baseUrl,
        defaultModel: saved.model,
        credential: "",
      });
      if (selectedTemplate?.protocol.authPlacement === "none") {
        setStatus("Provider 已保存；该模板无需会话凭据。");
      } else if (submittedCredential) {
        setStatus("Provider 已保存；新 API Key 已载入本次应用会话，退出即清除。");
      } else if (wasExisting) {
        setStatus("Provider 已保存；之前的会话 API Key 已清除，如需继续调用请重新输入并保存。");
      } else {
        setStatus("Provider 已保存；当前没有会话 API Key，如需调用请重新输入并保存。");
      }
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "保存 Provider 失败。");
    } finally {
      setSaving(false);
    }
  };

  const testConnection = async () => {
    setError(null);
    setStatus(null);
    const endpointError = validateEndpoint(draft.baseUrl);
    if (endpointError) {
      setError(endpointError);
      return;
    }
    if (!draft.id) {
      setError("请先保存 Provider，再测试 Rust Core 到端点的连接。");
      return;
    }
    if (connectionTestNeedsSave) {
      setError("当前配置尚未保存。请先保存更改，再测试 Rust Core 到最新端点的连接。");
      return;
    }
    setTesting(true);
    try {
      const result = await bridge.testProviderConnection({ providerProfileId: draft.id });
      const connection = result as { ok: boolean; message?: string };
      if (!connection.ok) throw new Error(connection.message || "端点未通过连接测试。");
      setStatus(connection.message || "连接成功。");
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "连接测试失败。");
    } finally {
      setTesting(false);
    }
  };

  return (
    <section className="provider-settings" aria-labelledby="provider-settings-title">
      <aside className="provider-settings__list">
        <header>
          <div>
            <span>模型端点</span>
            <h2 id="provider-settings-title">Providers</h2>
          </div>
          <button
            type="button"
            aria-label="新建 Provider"
            onClick={() => {
              const firstAvailable = templates.find((template) => template.runtimeAvailable);
              setDraft({
                ...emptyDraft,
                providerId: firstAvailable?.providerId ?? "",
                baseUrl: firstAvailable?.defaultBaseUrl ?? "",
              });
              setError(null);
              setStatus(null);
            }}
          >
            <Plus size={16} />
          </button>
        </header>
        {loading && <p className="provider-settings__empty">正在读取本地配置…</p>}
        {!loading && profiles.length === 0 && (
          <p className="provider-settings__empty">还没有 Provider。新建一个云端兼容端点，或连接本机 Ollama。</p>
        )}
        <div className="provider-settings__profiles">
          {profiles.map((profile) => (
            <button
              type="button"
              key={profile.id}
              className={draft.id === profile.id ? "is-active" : ""}
              onClick={() => editProfile(profile)}
            >
              <span className="provider-settings__profile-icon">
                {isLocalEndpoint(profile.baseUrl) ? <HardDrive size={15} /> : <Cloud size={15} />}
              </span>
              <span>
                <strong>{profile.name}</strong>
                <small>{profile.model}</small>
              </span>
            </button>
          ))}
        </div>
      </aside>

      <form className="provider-settings__form" onSubmit={submit}>
        <header>
          <span>Provider Profile</span>
          <h2>{draft.id ? "编辑模型端点" : "连接模型端点"}</h2>
          <p>网络请求只由 Rust Core 发起。前端不会读取或持久化你的 API Key。</p>
        </header>

        <div className="provider-settings__boundary">
          <LocalDataBadge />
          <span aria-hidden="true">→</span>
          <div>
            <strong>本轮模型调用</strong>
            <span>选中的 Context 会发送到 {parsedHost}</span>
          </div>
        </div>
        <p className="provider-settings__boundary-copy">
          工作区内容保存在本机；只有每轮凭证中明确纳入的 Context 会离开应用的数据边界。
        </p>

        <div className="provider-settings__grid">
          <label>
            <span>名称</span>
            <input value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="例如：团队兼容网关" />
          </label>
          <label>
            <span>Provider 模板</span>
            <select
              aria-label="Provider 模板"
              value={draft.providerId}
              disabled={Boolean(draft.id)}
              aria-describedby={draft.id ? "provider-template-lock-note" : undefined}
              onChange={(event) => {
                if (draft.id) return;
                const template = templates.find(
                  (candidate) => candidate.providerId === event.target.value,
                );
                if (!template) return;
                setDraft({
                  ...draft,
                  providerId: template.providerId,
                  baseUrl: template.defaultBaseUrl,
                  defaultModel: "",
                  credential: "",
                });
              }}
            >
              <option value="" disabled>请选择模板</option>
              {templates.map((template) => (
                <option
                  key={template.providerId}
                  value={template.providerId}
                >
                  {template.displayName}{template.runtimeAvailable ? "" : "（即将支持）"}
                </option>
              ))}
            </select>
            {draft.id && (
              <small id="provider-template-lock-note" className="provider-settings__field-note">
                已保存 Profile 的模板不可更改；如需切换协议，请新建 Provider。
              </small>
            )}
          </label>
          {selectedTemplate && (
            <div className="provider-settings__protocol provider-settings__wide" aria-label="协议说明">
              <span>流协议 <strong>{selectedTemplate.protocol.streamProtocol}</strong></span>
              <span>
                认证方式 <strong>{describeAuth(selectedTemplate)}</strong>
              </span>
            </div>
          )}
          <label className="provider-settings__wide">
            <span>Base URL</span>
            <input value={draft.baseUrl} onChange={(event) => setDraft({ ...draft, baseUrl: event.target.value })} placeholder="https://api.example.com/v1" inputMode="url" />
          </label>
          <label>
            <span>模型</span>
            <input value={draft.defaultModel} onChange={(event) => setDraft({ ...draft, defaultModel: event.target.value })} placeholder="gpt-4.1 或 qwen3:14b" />
          </label>
          <label>
            <span>API Key（仅本次会话）</span>
            <span className="provider-settings__secret">
              <KeyRound size={14} aria-hidden="true" />
              <input
                type="password"
                aria-label="API Key（仅本次会话）"
                aria-describedby={draft.id && selectedTemplate?.protocol.authPlacement !== "none" ? "provider-credential-reset-note" : undefined}
                autoComplete="off"
                disabled={selectedTemplate?.protocol.authPlacement === "none"}
                value={draft.credential}
                onChange={(event) => setDraft({ ...draft, credential: event.target.value })}
                placeholder={selectedTemplate?.protocol.authPlacement === "none" ? "通常不需要" : "退出应用后清除"}
              />
            </span>
            {draft.id && selectedTemplate?.protocol.authPlacement !== "none" && (
              <small id="provider-credential-reset-note" className="provider-settings__field-note">
                保存 Profile 会清除旧会话凭据；需要继续使用时，请在本次保存中重新输入。
              </small>
            )}
          </label>
        </div>

        {draft.baseUrl && (
          <div className="provider-settings__preview">
            <ProviderDestination
              name={draft.name || "未命名 Provider"}
              baseUrl={draft.baseUrl}
              local={isLocalEndpoint(draft.baseUrl)}
            />
          </div>
        )}

        {error && <p className="provider-settings__message is-error" role="alert">{error}</p>}
        {status && <p className="provider-settings__message is-success" role="status"><Check size={14} /> {status}</p>}

        <footer>
          <Button
            type="button"
            tone="quiet"
            icon={testing ? <LoaderCircle className="tf-spin" size={15} /> : <PlugZap size={15} />}
            disabled={testing || saving || connectionTestNeedsSave}
            onClick={testConnection}
          >
            {testing ? "测试中" : !draft.id ? "请先保存 Provider" : connectionTestNeedsSave ? "请先保存更改" : "测试连接"}
          </Button>
          <Button
            type="submit"
            tone="primary"
            disabled={saving || testing || !selectedTemplate?.runtimeAvailable}
          >
            {saving ? "保存中" : selectedTemplate && !selectedTemplate.runtimeAvailable ? "协议即将支持" : "保存 Provider"}
          </Button>
        </footer>
      </form>
    </section>
  );
}
