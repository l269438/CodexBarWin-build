import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeftRight,
  Box,
  CheckCircle2,
  Folder,
  KeyRound,
  MoreHorizontal,
  Plus,
  Power,
  RefreshCw,
  Save,
  Shield,
  Trash2,
} from "lucide-react";
import "./styles.css";

type ApiFormat = "open_ai_chat" | "open_ai_responses";

interface Provider {
  id: string;
  name: string;
  baseUrl: string;
  apiKey: string;
  model: string;
  apiFormat: ApiFormat;
}

interface AppConfig {
  providers: Provider[];
  currentProviderId?: string | null;
  proxyPort: number;
}

interface ProxyStatus {
  running: boolean;
  port: number;
  currentProviderId?: string | null;
}

type CodexActiveSource =
  | { kind: "liveSystem" }
  | { kind: "managedAccount"; id: string };

interface VisibleCodexAccount {
  id: string;
  email: string;
  workspaceLabel?: string | null;
  workspaceAccountId?: string | null;
  selectionSource: CodexActiveSource;
  isActive: boolean;
  isLive: boolean;
}

interface CodexVisibleAccountProjection {
  accounts: VisibleCodexAccount[];
  hasUnreadableStore: boolean;
}

interface CodexUsageWindow {
  usedPercent: number;
  remainingPercent: number;
  resetAt: number;
  limitWindowSeconds: number;
}

interface CodexUsageSummary {
  session?: CodexUsageWindow | null;
  weekly?: CodexUsageWindow | null;
  plan?: string | null;
  accountEmail?: string | null;
  fetchedAt: number;
}

interface AccountUsageSnapshot {
  sessionPercent: number | null;
  weeklyPercent: number | null;
  status: "refreshing" | "ready" | "unavailable";
}

type Workspace = "accounts" | "api";

const emptyProvider = (): Provider => ({
  id: crypto.randomUUID(),
  name: "",
  baseUrl: "",
  apiKey: "",
  model: "",
  apiFormat: "open_ai_responses",
});

function providerInitial(name: string) {
  return name.trim().slice(0, 1).toUpperCase() || "P";
}

function providerKind(name: string) {
  const lower = name.toLowerCase();
  if (lower.includes("openai")) return "openai";
  if (lower.includes("deepseek")) return "deepseek";
  if (lower.includes("anthropic") || lower.includes("claude")) return "anthropic";
  return "custom";
}

function providerDisplayIcon(name: string) {
  const kind = providerKind(name);
  if (kind === "openai") return "◎";
  if (kind === "deepseek") return "D";
  if (kind === "anthropic") return "AI";
  return providerInitial(name);
}

let browserConfig: AppConfig = {
  providers: [],
  currentProviderId: null,
  proxyPort: 15721,
};

let browserStatus: ProxyStatus = {
  running: false,
  port: 15721,
  currentProviderId: null,
};

let browserAccounts: CodexVisibleAccountProjection = {
  accounts: [],
  hasUnreadableStore: false,
};

function hasTauriRuntime() {
  return "__TAURI_INTERNALS__" in window;
}

async function callCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (hasTauriRuntime()) {
    return invoke<T>(command, args);
  }

  switch (command) {
    case "get_app_config":
      return structuredClone(browserConfig) as T;
    case "get_proxy_status":
      return structuredClone(browserStatus) as T;
    case "load_account_projection":
      return structuredClone(browserAccounts) as T;
    case "load_account_usage": {
      const accountId = (args?.accountId as string | undefined) ?? "live";
      const seed = accountId === "live" ? 32 : accountId.length * 7;
      return {
        session: {
          usedPercent: Math.min(93, seed % 100),
          remainingPercent: Math.max(7, 100 - (seed % 100)),
          resetAt: Math.floor(Date.now() / 1000) + 3600,
          limitWindowSeconds: 18000,
        },
        weekly: {
          usedPercent: Math.min(88, (seed + 24) % 100),
          remainingPercent: Math.max(12, 100 - ((seed + 24) % 100)),
          resetAt: Math.floor(Date.now() / 1000) + 604800,
          limitWindowSeconds: 604800,
        },
        plan: "preview",
        accountEmail:
          browserAccounts.accounts.find((account) => account.id === accountId)?.email ??
          "live@example.com",
        fetchedAt: Math.floor(Date.now() / 1000),
      } as T;
    }
    case "save_provider": {
      const provider = args?.provider as Provider;
      const index = browserConfig.providers.findIndex((item) => item.id === provider.id);
      if (index >= 0) browserConfig.providers[index] = provider;
      else browserConfig.providers.push(provider);
      return structuredClone(browserConfig) as T;
    }
    case "switch_provider":
      browserConfig.currentProviderId = args?.id as string;
      browserStatus.currentProviderId = browserConfig.currentProviderId;
      return structuredClone(browserConfig) as T;
    case "write_codex_takeover":
      return undefined as T;
    case "delete_provider": {
      if (browserConfig.providers.length <= 1) {
        throw new Error("cannot delete the last provider");
      }
      const id = args?.id as string;
      browserConfig.providers = browserConfig.providers.filter((item) => item.id !== id);
      if (browserConfig.currentProviderId === id) {
        browserConfig.currentProviderId = browserConfig.providers[0]?.id ?? null;
      }
      return structuredClone(browserConfig) as T;
    }
    case "start_proxy":
      if (!browserConfig.currentProviderId) {
        throw new Error("Add and select a provider before starting the proxy");
      }
      browserStatus = {
        running: true,
        port: browserConfig.proxyPort,
        currentProviderId: browserConfig.currentProviderId,
      };
      return structuredClone(browserStatus) as T;
    case "stop_proxy":
      browserStatus = {
        running: false,
        port: browserConfig.proxyPort,
        currentProviderId: browserConfig.currentProviderId,
      };
      return structuredClone(browserStatus) as T;
    case "import_current_account":
      throw new Error("Desktop runtime required to import the current Codex account");
    case "add_managed_account":
      throw new Error("Desktop runtime required to add a Codex account");
    case "switch_account": {
      const accountId = args?.accountId as string;
      browserAccounts.accounts = browserAccounts.accounts.map((account) => ({
        ...account,
        isActive: account.id === accountId,
      }));
      return undefined as T;
    }
    case "remove_managed_account": {
      const accountId = args?.accountId as string;
      browserAccounts.accounts = browserAccounts.accounts.filter(
        (account) => account.id !== accountId,
      );
      return structuredClone(browserAccounts) as T;
    }
    case "refresh_managed_account":
      return structuredClone(browserAccounts) as T;
    case "open_codex_home":
      return undefined as T;
    default:
      throw new Error(`Unsupported preview command: ${command}`);
  }
}

function formatUsagePercent(value: number | null) {
  return value === null ? "--" : `${Math.round(value)}%`;
}

function usageLabel(usage: AccountUsageSnapshot | undefined) {
  if (!usage || usage.status === "refreshing") return "用量刷新中";
  if (usage.status === "unavailable") return "用量暂不可用";
  return `会话 ${formatUsagePercent(usage.sessionPercent)} · 每周 ${formatUsagePercent(
    usage.weeklyPercent,
  )}`;
}

function usageFill(value: number | null) {
  if (value === null) return "0%";
  return `${Math.max(0, Math.min(100, Math.round(value)))}%`;
}

function accountInitial(email: string) {
  return email.trim().slice(0, 1).toUpperCase() || "A";
}

function formatWorkspaceLabel(label: string | null | undefined) {
  const normalized = label?.trim();
  if (!normalized) return "托管目录";
  if (normalized === "Personal") return "个人";
  if (normalized === "Work") return "工作";
  return normalized;
}

function App() {
  const [config, setConfig] = React.useState<AppConfig | null>(null);
  const [status, setStatus] = React.useState<ProxyStatus | null>(null);
  const [accounts, setAccounts] = React.useState<CodexVisibleAccountProjection | null>(null);
  const [accountUsage, setAccountUsage] = React.useState<Record<string, AccountUsageSnapshot>>({});
  const [activeWorkspace, setActiveWorkspace] = React.useState<Workspace>("accounts");
  const [selectedId, setSelectedId] = React.useState<string>("");
  const [draft, setDraft] = React.useState<Provider | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [message, setMessage] = React.useState("");
  const [accountMessage, setAccountMessage] = React.useState("");

  const load = React.useCallback(async () => {
    const [nextConfig, nextStatus, nextAccounts] = await Promise.all([
      callCommand<AppConfig>("get_app_config"),
      callCommand<ProxyStatus>("get_proxy_status"),
      callCommand<CodexVisibleAccountProjection>("load_account_projection"),
    ]);
    setConfig(nextConfig);
    setStatus(nextStatus);
    setAccounts(nextAccounts);
    setSelectedId((currentSelectedId) => {
      const selectedStillExists = nextConfig.providers.some(
        (provider) => provider.id === currentSelectedId,
      );
      const id = selectedStillExists
        ? currentSelectedId
        : nextConfig.currentProviderId || nextConfig.providers[0]?.id || "";
      return id;
    });
  }, []);

  React.useEffect(() => {
    void load();
  }, [load]);

  React.useEffect(() => {
    if (!config || !selectedId) return;
    setDraft((currentDraft) => {
      const savedProvider = config.providers.find((provider) => provider.id === selectedId);
      return savedProvider ?? (currentDraft?.id === selectedId ? currentDraft : null);
    });
  }, [config, selectedId]);

  const accountIds = React.useMemo(
    () => accounts?.accounts.map((account) => account.id) ?? [],
    [accounts],
  );

  React.useEffect(() => {
    if (!accountIds.length) {
      setAccountUsage({});
      return;
    }

    let cancelled = false;
    setAccountUsage((current) => {
      const next: Record<string, AccountUsageSnapshot> = {};
      for (const accountId of accountIds) {
        next[accountId] =
          current[accountId]?.status === "ready"
            ? current[accountId]
            : { sessionPercent: null, weeklyPercent: null, status: "refreshing" };
      }
      return next;
    });

    for (const accountId of accountIds) {
      void callCommand<CodexUsageSummary>("load_account_usage", { accountId })
        .then((summary) => {
          if (cancelled) return;
          setAccountUsage((current) => ({
            ...current,
            [accountId]: {
              sessionPercent: summary.session?.usedPercent ?? null,
              weeklyPercent: summary.weekly?.usedPercent ?? null,
              status: "ready",
            },
          }));
        })
        .catch(() => {
          if (cancelled) return;
          setAccountUsage((current) => ({
            ...current,
            [accountId]: {
              sessionPercent: null,
              weeklyPercent: null,
              status: "unavailable",
            },
          }));
        });
    }

    return () => {
      cancelled = true;
    };
  }, [accountIds.join("\u0000")]);

  async function run<T>(action: () => Promise<T>, success: string) {
    setBusy(true);
    setMessage("");
    try {
      const result = await action();
      setMessage(success);
      return result;
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
      throw error;
    } finally {
      setBusy(false);
    }
  }

  async function runAccount<T>(action: () => Promise<T>, success: string) {
    setBusy(true);
    setAccountMessage("");
    try {
      const result = await action();
      setAccountMessage(success);
      return result;
    } catch (error) {
      setAccountMessage(error instanceof Error ? error.message : String(error));
      throw error;
    } finally {
      setBusy(false);
    }
  }

  function validateDraftProvider(provider: Provider) {
    const missing = [
      !provider.name.trim() ? "服务商名称" : "",
      !provider.baseUrl.trim() ? "接口地址" : "",
      !provider.model.trim() ? "模型" : "",
    ].filter(Boolean);
    if (missing.length) {
      setMessage(`保存前请填写：${missing.join("、")}`);
      return false;
    }
    return true;
  }

  async function saveDraft() {
    if (!draft) return;
    if (!validateDraftProvider(draft)) return;
    const next = await run(
      () => callCommand<AppConfig>("save_provider", { provider: draft }),
      "服务商已保存",
    );
    setConfig(next);
    setSelectedId(draft.id);
  }

  async function switchToSelected() {
    if (!draft) return;
    if (!validateDraftProvider(draft)) return;
    let provider = draft;
    if (!config?.providers.some((item) => item.id === draft.id)) {
      const saved = await run(
        () => callCommand<AppConfig>("save_provider", { provider: draft }),
        "服务商已保存",
      );
      setConfig(saved);
      provider = saved.providers.find((item) => item.id === draft.id) ?? draft;
    }
    const next = await run(
      () => callCommand<AppConfig>("switch_provider", { id: provider.id }),
      "当前服务商已切换",
    );
    setConfig(next);
    await load();
  }

  async function startProxy() {
    if (!currentProvider) {
      setMessage("请先添加并选择一个服务商");
      return;
    }
    const next = await run(
      () => callCommand<ProxyStatus>("start_proxy"),
      "本地 Codex 代理已启动",
    );
    setStatus(next);
  }

  async function stopProxy() {
    const next = await run(
      () => callCommand<ProxyStatus>("stop_proxy"),
      "本地 Codex 代理已停止，原配置已恢复",
    );
    setStatus(next);
  }

  async function deleteSelected() {
    if (!draft) return;
    if (!config?.providers.some((provider) => provider.id === draft.id)) {
      setDraft(null);
      setSelectedId(config?.currentProviderId || config?.providers[0]?.id || "");
      setMessage("草稿已丢弃");
      return;
    }
    const next = await run(
      () => callCommand<AppConfig>("delete_provider", { id: draft.id }),
      "服务商已删除",
    );
    setConfig(next);
    const nextId = next.currentProviderId || next.providers[0]?.id || "";
    setSelectedId(nextId);
  }

  async function importCurrentAccount() {
    const next = await runAccount(
      () => callCommand<CodexVisibleAccountProjection>("import_current_account"),
      "当前 Codex 账号已导入",
    );
    setAccounts(next);
  }

  async function addManagedAccount() {
    const next = await runAccount(
      () => callCommand<CodexVisibleAccountProjection>("add_managed_account"),
      "托管 Codex 账号已添加",
    );
    setAccounts(next);
  }

  async function switchAccount(accountId: string) {
    await runAccount(
      () => callCommand<void>("switch_account", { accountId }),
      "Codex 账号已切换",
    );
    await load();
  }

  async function removeManagedAccount(accountId: string) {
    const next = await runAccount(
      () => callCommand<CodexVisibleAccountProjection>("remove_managed_account", { accountId }),
      "托管账号已移除",
    );
    setAccounts(next);
  }

  async function refreshManagedAccount(accountId: string) {
    const next = await runAccount(
      () => callCommand<CodexVisibleAccountProjection>("refresh_managed_account", { accountId }),
      "托管账号已从当前登录刷新",
    );
    setAccounts(next);
  }

  function addProvider() {
    const provider = emptyProvider();
    setSelectedId(provider.id);
    setDraft(provider);
  }

  async function useProvider(provider: Provider) {
    setSelectedId(provider.id);
    setDraft(provider);
    const switched = await run(
      () => callCommand<AppConfig>("switch_provider", { id: provider.id }),
      "当前服务商已切换",
    );
    setConfig(switched);
    if (status?.running) {
      await callCommand<void>("write_codex_takeover");
    }
    await load();
  }

  async function openCodexHome() {
    await runAccount(() => callCommand<void>("open_codex_home"), "Codex 目录已打开");
  }

  const currentProvider = config?.providers.find(
    (provider) => provider.id === config.currentProviderId,
  );
  const activeAccount = accounts?.accounts.find((account) => account.isActive);
  const activeUsage = activeAccount ? accountUsage[activeAccount.id] : undefined;
  const routeUrl = `http://127.0.0.1:${config?.proxyPort ?? 15721}/v1`;
  const draftIsSaved = Boolean(
    draft && config?.providers.some((provider) => provider.id === draft.id),
  );
  const providerCount = config?.providers.length ?? 0;
  const accountCount = accounts?.accounts.length ?? 0;
  const managedCount = accounts?.accounts.filter((account) => !account.isLive).length ?? 0;
  const displayedProviders = config?.providers.slice(0, 3) ?? [];
  const displayedAccounts = accounts?.accounts.slice(0, 3) ?? [];
  const currentRouteLabel =
    (draft ?? currentProvider)?.apiFormat === "open_ai_chat" ? "聊天转换" : "响应直连";

  return (
    <main className="app-shell">
      <section className="tray-window" aria-label="CodexPilot">
        <header className="pilot-header">
          <div className="brand-lockup">
            <div className="brand-mark">CP</div>
            <div className="brand-copy">
              <h1>CodexPilot</h1>
              <p>{status?.running ? "代理运行中" : "单层控制台"}</p>
            </div>
          </div>
          <div className="status-stack">
            <strong>{status?.running ? "ON" : "OFF"}</strong>
            <span>{providerCount} API · {managedCount} 账号</span>
          </div>
        </header>

        <nav className="workspace-switcher" aria-label="工作区" role="tablist">
          <button
            aria-selected={activeWorkspace === "accounts"}
            className={activeWorkspace === "accounts" ? "active" : ""}
            onClick={() => setActiveWorkspace("accounts")}
            role="tab"
            type="button"
          >
            账号管理
          </button>
          <button
            aria-selected={activeWorkspace === "api"}
            className={activeWorkspace === "api" ? "active" : ""}
            onClick={() => setActiveWorkspace("api")}
            role="tab"
            type="button"
          >
            接入 API
          </button>
        </nav>

        {activeWorkspace === "accounts" ? (
          <section className="workspace-panel accounts-panel" aria-label="账号管理">
            <section className="focus-strip">
              <span className="focus-kicker">当前账号</span>
              <strong>{activeAccount?.email ?? "未检测到 Codex 账号"}</strong>
              <span className={`run-pill ${activeAccount ? "on" : ""}`}>
                <span />
                {activeAccount
                  ? activeAccount.isLive
                    ? "系统账号"
                    : "托管账号"
                  : "等待登录"}
              </span>
            </section>

            <section className="usage-panel" aria-label="用量概览">
              <div>
                <span>会话</span>
                <strong>{formatUsagePercent(activeUsage?.sessionPercent ?? null)}</strong>
                <i
                  style={
                    {
                      "--usage-fill": usageFill(activeUsage?.sessionPercent ?? null),
                    } as React.CSSProperties
                  }
                />
              </div>
              <div>
                <span>每周</span>
                <strong>{formatUsagePercent(activeUsage?.weeklyPercent ?? null)}</strong>
                <i
                  style={
                    {
                      "--usage-fill": usageFill(activeUsage?.weeklyPercent ?? null),
                    } as React.CSSProperties
                  }
                />
              </div>
            </section>

            <section className="toolbar-line" aria-label="账号操作">
              <button className="primary-button" disabled={busy} onClick={addManagedAccount} type="button">
                <Plus size={16} />
                添加账号
              </button>
              <button className="soft-button" disabled={busy} onClick={importCurrentAccount} type="button">
                导入当前
              </button>
              <button className="icon-button" disabled={busy} onClick={() => void load()} title="刷新" type="button">
                <RefreshCw size={17} />
              </button>
            </section>

            {accounts?.hasUnreadableStore && (
              <p className="message">托管账号存储不可读取。</p>
            )}

            <section className="compact-list account-table" aria-label="账号列表">
              {displayedAccounts.length ? (
                displayedAccounts.map((account) => {
                  const usage = accountUsage[account.id];
                  return (
                    <div className="account-row" key={account.id}>
                      <span className={`account-avatar ${account.isActive ? "active" : ""}`}>
                        {accountInitial(account.email)}
                      </span>
                      <span className="account-copy">
                        <strong>
                          {account.email}
                          {account.isActive ? <em>当前</em> : null}
                        </strong>
                        <small>
                          {account.isLive ? "系统登录" : formatWorkspaceLabel(account.workspaceLabel)} · {usageLabel(usage)}
                        </small>
                      </span>
                      <span className="account-menu">
                        {!account.isActive ? (
                          <button disabled={busy} onClick={() => switchAccount(account.id)} type="button">
                            切换
                          </button>
                        ) : null}
                        <button
                          disabled={busy}
                          onClick={() =>
                            account.isLive ? importCurrentAccount() : refreshManagedAccount(account.id)
                          }
                          title={account.isLive ? "导入当前" : "刷新账号"}
                          type="button"
                        >
                          <MoreHorizontal size={17} />
                        </button>
                      </span>
                    </div>
                  );
                })
              ) : (
                <p className="empty">还没有检测到 Codex 账号。</p>
              )}
            </section>

            <footer className="workspace-footer">
              <span>{accountCount > displayedAccounts.length ? `另有 ${accountCount - displayedAccounts.length} 个账号未展示` : "账号列表已就绪"}</span>
              <button onClick={() => void openCodexHome()} type="button">
                <Folder size={16} />
                Codex 目录
              </button>
            </footer>
            {accountMessage && <p className="message">{accountMessage}</p>}
          </section>
        ) : (
          <section className="workspace-panel api-panel" aria-label="接入 API">
            <section className="focus-strip api-focus">
              <span className="focus-kicker">当前服务商</span>
              <strong>{currentProvider?.name ?? "未配置服务商"}</strong>
              <span className={`run-pill ${status?.running ? "on" : ""}`}>
                <span />
                {status?.running ? "代理已启动" : "代理未启动"}
              </span>
            </section>

            <section className="api-route" aria-label="路由摘要">
              <div>
                <Box size={16} />
                <span>模型</span>
                <strong>{draft?.model || currentProvider?.model || "未填写"}</strong>
              </div>
              <div>
                <ArrowLeftRight size={16} />
                <span>模式</span>
                <strong>{draft || currentProvider ? currentRouteLabel : "未选择"}</strong>
              </div>
              <div>
                <Shield size={16} />
                <span>会话</span>
                <strong>保留</strong>
              </div>
            </section>

            <section className="toolbar-line" aria-label="API 操作">
              {status?.running ? (
                <button className="primary-button stop-button" disabled={busy} onClick={stopProxy} type="button">
                  <span />
                  停止代理
                </button>
              ) : (
                <button className="primary-button" disabled={busy} onClick={startProxy} type="button">
                  <Power size={16} />
                  启动代理
                </button>
              )}
              <button className="soft-button" disabled={busy} onClick={addProvider} type="button">
                <Plus size={16} />
                新服务商
              </button>
              <button className="icon-button" disabled={busy} onClick={() => void load()} title="刷新" type="button">
                <RefreshCw size={17} />
              </button>
            </section>

            <section className="compact-list provider-table" aria-label="服务商列表">
              {displayedProviders.length ? (
                displayedProviders.map((provider) => {
                  const active = provider.id === config?.currentProviderId;
                  return (
                    <button
                      className={`provider-row ${active ? "active" : ""}`}
                      disabled={busy}
                      key={provider.id}
                      onClick={() => void useProvider(provider)}
                      type="button"
                    >
                      <span className={`provider-logo ${providerKind(provider.name)}`}>
                        {providerDisplayIcon(provider.name)}
                      </span>
                      <span className="provider-copy">
                        <strong>{provider.name || "未命名服务商"}</strong>
                        <small>{provider.model || provider.baseUrl || "未填写模型"}</small>
                      </span>
                      <span className="provider-state">
                        {active ? <CheckCircle2 size={20} /> : <span>启用</span>}
                      </span>
                    </button>
                  );
                })
              ) : (
                <div className="empty-panel">
                  <strong>还没有服务商</strong>
                  <span>先添加 API 服务商，再启动本地代理。</span>
                </div>
              )}
            </section>

            <section className="api-editor" aria-label="服务商编辑">
              <label>
                服务商
                <input
                  value={draft?.name ?? ""}
                  placeholder="OpenAI / DeepSeek / 自定义"
                  onChange={(event) =>
                    draft && setDraft({ ...draft, name: event.currentTarget.value })
                  }
                />
              </label>
              <label>
                接口地址
                <input
                  value={draft?.baseUrl ?? ""}
                  placeholder="https://api.example.com/v1"
                  onChange={(event) =>
                    draft && setDraft({ ...draft, baseUrl: event.currentTarget.value })
                  }
                />
              </label>
              <label>
                密钥
                <span className="input-icon">
                  <KeyRound size={15} />
                  <input
                    type="password"
                    value={draft?.apiKey ?? ""}
                    placeholder="请输入密钥"
                    onChange={(event) =>
                      draft && setDraft({ ...draft, apiKey: event.currentTarget.value })
                    }
                  />
                </span>
              </label>
              <label>
                模型
                <input
                  value={draft?.model ?? ""}
                  placeholder="模型名称"
                  onChange={(event) =>
                    draft && setDraft({ ...draft, model: event.currentTarget.value })
                  }
                />
              </label>
              <label>
                格式
                <select
                  value={draft?.apiFormat ?? "open_ai_responses"}
                  onChange={(event) =>
                    draft &&
                    setDraft({ ...draft, apiFormat: event.currentTarget.value as ApiFormat })
                  }
                >
                  <option value="open_ai_responses">响应直连</option>
                  <option value="open_ai_chat">聊天转换</option>
                </select>
              </label>
            </section>

            <footer className="workspace-footer">
              <span>{routeUrl}</span>
              <div className="footer-actions">
                <button disabled={busy || !draft} onClick={saveDraft} type="button">
                  <Save size={15} />
                  保存
                </button>
                <button disabled={busy || !draft} onClick={switchToSelected} type="button">
                  启用
                </button>
                <button
                  className="danger-button"
                  disabled={
                    busy ||
                    !draft ||
                    (draftIsSaved && (config?.providers.length ?? 0) <= 1)
                  }
                  onClick={deleteSelected}
                  title={draftIsSaved ? "删除服务商" : "丢弃草稿"}
                  type="button"
                >
                  <Trash2 size={15} />
                </button>
              </div>
            </footer>
            {message && <p className="message">{message}</p>}
          </section>
        )}
      </section>
    </main>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
