import { Check, Cloud, HardDrive, KeyRound, LoaderCircle, PlugZap, Plus } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import type { DesktopBridge } from "../../platform/desktop-bridge";
import type { ProviderProfile } from "../../shared/contracts";
import { Button, LocalDataBadge, ProviderDestination } from "../../shared/ui";
import "./provider-settings.css";

type ProviderProfileView = ProviderProfile;

type Draft = {
  id?: string;
  name: string;
  dialect: ProviderProfileView["dialect"];
  baseUrl: string;
  defaultModel: string;
  credential: string;
};

const emptyDraft: Draft = {
  name: "",
  dialect: "openai-compatible",
  baseUrl: "",
  defaultModel: "",
  credential: "",
};

function isLoopbackUrl(url: URL) {
  return ["127.0.0.1", "localhost", "[::1]", "::1"].includes(url.hostname);
}

function validateEndpoint(rawUrl: string) {
  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    return "请输入完整的 Base URL，例如 https://api.example.com/v1。";
  }
  if (url.username || url.password) return "Base URL 不能包含用户名、密码或 API Key。";
  if (url.protocol === "http:" && !isLoopbackUrl(url)) return "远程端点必须使用 HTTPS。";
  if (url.protocol !== "https:" && url.protocol !== "http:") return "Provider 端点只支持 HTTP 或 HTTPS。";
  return null;
}

export function ProviderSettings({ bridge }: { bridge: DesktopBridge }) {
  const [profiles, setProfiles] = useState<ProviderProfileView[]>([]);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    bridge
      .listProviderProfiles()
      .then((nextProfiles) => {
        if (active) setProfiles(nextProfiles as ProviderProfileView[]);
      })
      .catch((reason: unknown) => {
        if (active) setError(reason instanceof Error ? reason.message : "无法读取 Provider 配置。");
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => { active = false; };
  }, [bridge]);

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
      name: profile.name,
      dialect: profile.dialect,
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
    if (!draft.defaultModel.trim()) {
      setError("请填写默认模型。");
      return;
    }

    setSaving(true);
    try {
      const saved = (await bridge.saveProviderProfile({
        id: draft.id,
        name: draft.name.trim(),
        dialect: draft.dialect,
        baseUrl: draft.baseUrl.trim().replace(/\/$/, ""),
        model: draft.defaultModel.trim(),
        isDefault: profiles.length === 0,
        parameters: {},
      })) as ProviderProfileView;
      if (draft.credential) {
        await bridge.setSessionCredential({
          providerProfileId: saved.id,
          credential: draft.credential,
        });
      }
      setProfiles((current) => {
        const withoutSaved = current.filter((profile) => profile.id !== saved.id);
        return [...withoutSaved, saved];
      });
      setDraft((current) => ({ ...current, id: saved.id, credential: "" }));
      setStatus("Provider 已保存；API Key 仅在本次应用会话中可用。");
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
          <button type="button" aria-label="新建 Provider" onClick={() => setDraft(emptyDraft)}>
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
                {profile.dialect === "ollama" ? <HardDrive size={15} /> : <Cloud size={15} />}
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
            <span>协议</span>
            <select
              value={draft.dialect}
              onChange={(event) => {
                const dialect = event.target.value as Draft["dialect"];
                setDraft({
                  ...draft,
                  dialect,
                  baseUrl: dialect === "ollama" && !draft.baseUrl ? "http://127.0.0.1:11434" : draft.baseUrl,
                });
              }}
            >
              <option value="openai-compatible">OpenAI-compatible</option>
              <option value="ollama">Ollama native</option>
            </select>
          </label>
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
              <input type="password" autoComplete="off" value={draft.credential} onChange={(event) => setDraft({ ...draft, credential: event.target.value })} placeholder={draft.dialect === "ollama" ? "通常不需要" : "退出应用后清除"} />
            </span>
          </label>
        </div>

        {draft.baseUrl && (
          <div className="provider-settings__preview">
            <ProviderDestination
              name={draft.name || "未命名 Provider"}
              baseUrl={draft.baseUrl}
              local={draft.dialect === "ollama" || (() => {
                try { return isLoopbackUrl(new URL(draft.baseUrl)); } catch { return false; }
              })()}
            />
          </div>
        )}

        {error && <p className="provider-settings__message is-error" role="alert">{error}</p>}
        {status && <p className="provider-settings__message is-success" role="status"><Check size={14} /> {status}</p>}

        <footer>
          <Button type="button" tone="quiet" icon={testing ? <LoaderCircle className="tf-spin" size={15} /> : <PlugZap size={15} />} disabled={testing || saving} onClick={testConnection}>
            {testing ? "测试中" : "测试连接"}
          </Button>
          <Button type="submit" tone="primary" disabled={saving || testing}>
            {saving ? "保存中" : "保存 Provider"}
          </Button>
        </footer>
      </form>
    </section>
  );
}
