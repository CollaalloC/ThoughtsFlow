import { Check, Cloud, HardDrive, KeyRound, LoaderCircle, PlugZap, Plus } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";
import { DesktopBridgeError, type DesktopBridge } from "../../platform/desktop-bridge";
import type {
  ListProviderModelsInput,
  ProviderModelInfo,
  ProviderProfile,
  ProviderTemplate,
} from "../../shared/contracts";
import { Button, LocalDataBadge, ProviderDestination } from "../../shared/ui";
import { SessionCredentialManager } from "./SessionCredentialManager";
import "./provider-settings.css";

type ProviderProfileView = ProviderProfile;
type ModelDiscoveryPhase = "idle" | "loading" | "success" | "error";

type Draft = {
  id?: string;
  providerId: string;
  name: string;
  baseUrl: string;
  defaultModel: string;
  credentialLabel: string;
  credential: string;
};

const emptyDraft: Draft = {
  providerId: "",
  name: "",
  baseUrl: "",
  defaultModel: "",
  credentialLabel: "Primary",
  credential: "",
};

interface ProviderSettingsProps {
  bridge: DesktopBridge;
  initialProviderProfileId?: string;
}

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

function modelOptionLabel(model: ProviderModelInfo) {
  const displayName = model.displayName.trim();
  return !displayName || displayName === model.id
    ? model.id
    : `${displayName} (${model.id})`;
}

function draftFromProfile(profile: ProviderProfileView): Draft {
  return {
    id: profile.id,
    providerId: profile.providerId,
    name: profile.name,
    baseUrl: profile.baseUrl,
    defaultModel: profile.model,
    credentialLabel: "Primary",
    credential: "",
  };
}

export function ProviderSettings({
  bridge,
  initialProviderProfileId,
}: ProviderSettingsProps) {
  const [profiles, setProfiles] = useState<ProviderProfileView[]>([]);
  const [templates, setTemplates] = useState<ProviderTemplate[]>([]);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [models, setModels] = useState<ProviderModelInfo[]>([]);
  const [modelDiscoveryPhase, setModelDiscoveryPhase] =
    useState<ModelDiscoveryPhase>("idle");
  const [modelDiscoveryError, setModelDiscoveryError] = useState<{
    message: string;
    retryable: boolean;
  } | null>(null);
  const modelDiscoveryRequest = useRef(0);
  const [credentialManagerRevision, setCredentialManagerRevision] = useState(0);

  const clearModelDiscovery = () => {
    modelDiscoveryRequest.current += 1;
    setModels([]);
    setModelDiscoveryPhase("idle");
    setModelDiscoveryError(null);
  };

  useEffect(() => {
    let active = true;
    Promise.all([bridge.listProviderProfiles(), bridge.listProviderTemplates()])
      .then(([nextProfiles, nextTemplates]) => {
        if (!active) return;
        setProfiles(nextProfiles as ProviderProfileView[]);
        setTemplates(nextTemplates);
        const initialProfile = initialProviderProfileId
          ? nextProfiles.find((profile) => profile.id === initialProviderProfileId)
          : undefined;
        if (initialProfile) {
          setDraft(draftFromProfile(initialProfile));
          return;
        }
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
    return () => {
      active = false;
      modelDiscoveryRequest.current += 1;
    };
  }, [bridge, initialProviderProfileId]);

  const selectedTemplate = useMemo(
    () => templates.find((template) => template.providerId === draft.providerId),
    [draft.providerId, templates],
  );
  const selectedDiscoveredModel = useMemo(
    () => models.find((model) => model.id === draft.defaultModel),
    [draft.defaultModel, models],
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
    clearModelDiscovery();
    setDraft(draftFromProfile(profile));
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
    if (draft.credential && !draft.credentialLabel.trim()) {
      setError("请填写初始凭据标签。");
      return;
    }

    setSaving(true);
    try {
      const wasExisting = Boolean(draft.id);
      const submittedCredential = draft.credential;
      const submittedCredentialLabel = draft.credentialLabel.trim();
      const persisted = draft.id
        ? profiles.find((profile) => profile.id === draft.id)
        : undefined;
      const targetChanged = Boolean(
        persisted
        && (
          persisted.providerId !== draft.providerId
          || normalizeBaseUrl(persisted.baseUrl) !== normalizeBaseUrl(draft.baseUrl)
        ),
      );
      if (submittedCredential) {
        setDraft((current) => ({ ...current, credential: "" }));
      }
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
          ? bridge.saveProviderProfile(profileInput, {
              label: submittedCredentialLabel,
              credential: submittedCredential,
            })
          : bridge.saveProviderProfile(profileInput)
      )) as ProviderProfileView;
      setProfiles((current) => {
        const withoutSaved = current.filter((profile) => profile.id !== saved.id);
        return [...withoutSaved, saved];
      });
      clearModelDiscovery();
      setDraft(draftFromProfile(saved));
      setCredentialManagerRevision((current) => current + 1);
      if (selectedTemplate?.protocol.authPlacement === "none") {
        setStatus("Provider 已保存；该模板无需会话凭据。");
      } else if (submittedCredential) {
        setStatus(`Provider 已保存；命名凭据“${submittedCredentialLabel}”已载入 Rust 进程内存，退出即清除。`);
      } else if (wasExisting && targetChanged) {
        setStatus("Provider 已保存；端点身份已变更，旧会话凭据已在保存成功后清除。");
      } else if (wasExisting) {
        setStatus("Provider 已保存；端点身份未变，现有会话凭据已保留。");
      } else {
        setStatus("Provider 已保存；当前没有会话凭据，可在下方会话凭据管理器中添加。");
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

  const discoverModels = async () => {
    setError(null);
    setStatus(null);
    setModelDiscoveryError(null);

    const endpointError = validateEndpoint(draft.baseUrl);
    if (endpointError) {
      setModelDiscoveryPhase("error");
      setModelDiscoveryError({ message: endpointError, retryable: false });
      return;
    }
    if (!selectedTemplate) {
      setModelDiscoveryPhase("error");
      setModelDiscoveryError({ message: "请选择 Provider 模板。", retryable: false });
      return;
    }
    if (!selectedTemplate.protocol.modelsEndpoint) {
      setModelDiscoveryPhase("error");
      setModelDiscoveryError({
        message: "该 Provider 模板没有可用的模型目录端点。",
        retryable: false,
      });
      return;
    }

    const normalizedBaseUrl = normalizeBaseUrl(draft.baseUrl);
    const persisted = draft.id
      ? profiles.find((profile) => profile.id === draft.id)
      : undefined;
    const canUseSavedProfile = Boolean(
      draft.id
      && persisted
      && persisted.providerId === draft.providerId
      && normalizeBaseUrl(persisted.baseUrl) === normalizedBaseUrl
      && draft.credential.length === 0,
    );
    const input: ListProviderModelsInput = canUseSavedProfile && draft.id
      ? { providerProfileId: draft.id }
      : {
          draft: {
            providerId: draft.providerId,
            baseUrl: normalizedBaseUrl,
            ...(draft.credential && selectedTemplate.protocol.authPlacement !== "none"
              ? { sessionCredential: draft.credential }
              : {}),
          },
        };
    const requestId = ++modelDiscoveryRequest.current;
    setModels([]);
    setModelDiscoveryPhase("loading");

    try {
      const discovered = await bridge.listProviderModels(input);
      if (requestId !== modelDiscoveryRequest.current) return;
      setModels(discovered);
      setModelDiscoveryPhase("success");
    } catch (reason) {
      if (requestId !== modelDiscoveryRequest.current) return;
      setModelDiscoveryPhase("error");
      setModelDiscoveryError({
        message: reason instanceof Error ? reason.message : "发现模型失败。",
        retryable: reason instanceof DesktopBridgeError && reason.retryable,
      });
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
              clearModelDiscovery();
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

      <div className="provider-settings__details">
        <form className="provider-settings__form" onSubmit={submit}>
          <header>
            <span>Provider Profile</span>
            <h2>{draft.id ? "编辑模型端点" : "连接模型端点"}</h2>
            <p>
              网络请求只由 Rust Core 发起。新建时，API Key 会短暂存在 WebView 表单；
              提交后只保留在 Rust 进程内存，不写入 SQLite 或前端持久状态。
            </p>
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
                clearModelDiscovery();
                setDraft({
                  ...draft,
                  providerId: template.providerId,
                  baseUrl: template.defaultBaseUrl,
                  defaultModel: "",
                  credentialLabel: "Primary",
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
            <input
              aria-label="Base URL"
              value={draft.baseUrl}
              onChange={(event) => {
                clearModelDiscovery();
                setDraft({ ...draft, baseUrl: event.target.value });
              }}
              placeholder="https://api.example.com/v1"
              inputMode="url"
            />
          </label>
          <div className="provider-settings__field provider-settings__model-field">
            <label htmlFor="provider-model-select">模型</label>
            <span className="provider-settings__model-controls">
              <input
                id="provider-model-select"
                list="provider-model-options"
                aria-label="模型"
                aria-describedby="provider-model-discovery-boundary"
                value={draft.defaultModel}
                onChange={(event) =>
                  setDraft({ ...draft, defaultModel: event.target.value })
                }
                placeholder="发现后选择，或手动输入模型 ID"
                autoComplete="off"
              />
              <datalist id="provider-model-options">
                {models.map((model) => (
                  <option key={model.id} value={model.id}>
                    {modelOptionLabel(model)}
                  </option>
                ))}
              </datalist>
              <button
                type="button"
                className="provider-settings__discover"
                disabled={
                  modelDiscoveryPhase === "loading"
                  || saving
                  || testing
                  || !selectedTemplate
                  || !draft.baseUrl.trim()
                }
                onClick={discoverModels}
              >
                {modelDiscoveryPhase === "loading" && (
                  <LoaderCircle className="tf-spin" size={13} aria-hidden="true" />
                )}
                {modelDiscoveryPhase === "loading" ? "发现中" : "发现模型"}
              </button>
            </span>
            <small
              id="provider-model-discovery-boundary"
              className="provider-settings__field-note"
            >
              “发现模型”只读取模型目录元数据，不发送工作区 Context。远程目录仅向 {parsedHost}
              发起 GET；内置审核列表不会联网。
            </small>
            {selectedDiscoveredModel && (
              <small className="provider-settings__model-metadata">
                目录元数据：{selectedDiscoveredModel.displayName.trim() || selectedDiscoveredModel.id}
                {" · Context "}
                {selectedDiscoveredModel.contextWindow === null
                  ? "未声明"
                  : selectedDiscoveredModel.contextWindow.toLocaleString("en-US")}
                {" · Tools "}
                {selectedDiscoveredModel.supportsTools === null
                  ? "未声明"
                  : selectedDiscoveredModel.supportsTools ? "支持" : "不支持"}
              </small>
            )}
            {modelDiscoveryPhase === "success" && models.length > 0 && (
              <small className="provider-settings__discovery-state" role="status">
                已发现 {models.length} 个模型；选择后仍需保存 Provider 才会生效。
              </small>
            )}
            {modelDiscoveryPhase === "success" && models.length === 0 && (
              <small className="provider-settings__discovery-state" role="status">
                端点返回了空模型列表。可稍后重新发现。
              </small>
            )}
            {modelDiscoveryPhase === "error" && modelDiscoveryError && (
              <span className="provider-settings__discovery-error" role="alert">
                <span>
                  {modelDiscoveryError.message}
                  {modelDiscoveryError.retryable ? " 这是临时错误，可以重试。" : ""}
                </span>
                {modelDiscoveryError.retryable && (
                  <button type="button" onClick={discoverModels}>重试发现模型</button>
                )}
              </span>
            )}
          </div>
          {!draft.id && selectedTemplate?.protocol.authPlacement !== "none" && (
            <>
              <label>
                <span>初始凭据标签</span>
                <input
                  aria-label="初始凭据标签"
                  autoComplete="off"
                  value={draft.credentialLabel}
                  onChange={(event) =>
                    setDraft({ ...draft, credentialLabel: event.target.value })
                  }
                  placeholder="例如：Primary"
                />
              </label>
              <label>
                <span>API Key（仅本次会话）</span>
                <span className="provider-settings__secret">
                  <KeyRound size={14} aria-hidden="true" />
                  <input
                    type="password"
                    aria-label="API Key（仅本次会话）"
                    autoComplete="new-password"
                    value={draft.credential}
                    onChange={(event) => {
                      clearModelDiscovery();
                      setDraft({ ...draft, credential: event.target.value });
                    }}
                    placeholder="提交后从 WebView 表单清除"
                  />
                </span>
              </label>
            </>
          )}
          {draft.id && selectedTemplate?.protocol.authPlacement !== "none" && (
            <p className="provider-settings__credential-policy provider-settings__wide">
              名称、模型和参数更新会保留现有会话凭据；Provider 身份或 Base URL
              变更只会在保存成功后清除旧凭据。已保存的 Secret 不会返回 WebView。
            </p>
          )}
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
        {draft.id && selectedTemplate?.protocol.authPlacement !== "none" && (
          <div className="provider-settings__credential-manager">
            <SessionCredentialManager
              key={`${draft.id}:${credentialManagerRevision}`}
              bridge={bridge}
              providerProfileId={draft.id}
              disabled={saving || testing}
            />
          </div>
        )}
      </div>
    </section>
  );
}
