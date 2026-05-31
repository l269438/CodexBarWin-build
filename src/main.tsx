import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeftRight,
  Box,
  CheckCircle2,
  ChevronDown,
  ExternalLink,
  Folder,
  KeyRound,
  MoreHorizontal,
  Plus,
  Power,
  RefreshCw,
  Save,
  Settings,
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
  if (!usage || usage.status === "refreshing") return "Usage refreshing";
  if (usage.status === "unavailable") return "Usage unavailable";
  return `Session ${formatUsagePercent(usage.sessionPercent)} · Weekly ${formatUsagePercent(
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

function App() {
  const [config, setConfig] = React.useState<AppConfig | null>(null);
  const [status, setStatus] = React.useState<ProxyStatus | null>(null);
  const [accounts, setAccounts] = React.useState<CodexVisibleAccountProjection | null>(null);
  const [accountUsage, setAccountUsage] = React.useState<Record<string, AccountUsageSnapshot>>({});
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
      !provider.name.trim() ? "name" : "",
      !provider.baseUrl.trim() ? "base URL" : "",
      !provider.model.trim() ? "model" : "",
    ].filter(Boolean);
    if (missing.length) {
      setMessage(`Fill provider ${missing.join(", ")} before saving`);
      return false;
    }
    return true;
  }

  async function saveDraft() {
    if (!draft) return;
    if (!validateDraftProvider(draft)) return;
    const next = await run(
      () => callCommand<AppConfig>("save_provider", { provider: draft }),
      "Provider saved",
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
        "Provider saved",
      );
      setConfig(saved);
      provider = saved.providers.find((item) => item.id === draft.id) ?? draft;
    }
    const next = await run(
      () => callCommand<AppConfig>("switch_provider", { id: provider.id }),
      "Current provider switched",
    );
    setConfig(next);
    await load();
  }

  async function startProxy() {
    if (!currentProvider) {
      setMessage("Add and select a provider before starting the proxy");
      return;
    }
    const next = await run(
      () => callCommand<ProxyStatus>("start_proxy"),
      "Local Codex proxy is running",
    );
    setStatus(next);
  }

  async function stopProxy() {
    const next = await run(
      () => callCommand<ProxyStatus>("stop_proxy"),
      "Local Codex proxy stopped; original config restored",
    );
    setStatus(next);
  }

  async function deleteSelected() {
    if (!draft) return;
    if (!config?.providers.some((provider) => provider.id === draft.id)) {
      setDraft(null);
      setSelectedId(config?.currentProviderId || config?.providers[0]?.id || "");
      setMessage("Draft provider discarded");
      return;
    }
    const next = await run(
      () => callCommand<AppConfig>("delete_provider", { id: draft.id }),
      "Provider deleted",
    );
    setConfig(next);
    const nextId = next.currentProviderId || next.providers[0]?.id || "";
    setSelectedId(nextId);
  }

  async function importCurrentAccount() {
    const next = await runAccount(
      () => callCommand<CodexVisibleAccountProjection>("import_current_account"),
      "Current Codex account imported",
    );
    setAccounts(next);
  }

  async function addManagedAccount() {
    const next = await runAccount(
      () => callCommand<CodexVisibleAccountProjection>("add_managed_account"),
      "Managed Codex account added",
    );
    setAccounts(next);
  }

  async function switchAccount(accountId: string) {
    await runAccount(
      () => callCommand<void>("switch_account", { accountId }),
      "Codex account switched",
    );
    await load();
  }

  async function removeManagedAccount(accountId: string) {
    const next = await runAccount(
      () => callCommand<CodexVisibleAccountProjection>("remove_managed_account", { accountId }),
      "Managed Codex account removed",
    );
    setAccounts(next);
  }

  async function refreshManagedAccount(accountId: string) {
    const next = await runAccount(
      () => callCommand<CodexVisibleAccountProjection>("refresh_managed_account", { accountId }),
      "Managed Codex account refreshed from live auth",
    );
    setAccounts(next);
  }

  function addProvider() {
    const provider = emptyProvider();
    setSelectedId(provider.id);
    setDraft(provider);
    window.setTimeout(openAdvancedDetails, 0);
  }

  async function useProvider(provider: Provider) {
    setSelectedId(provider.id);
    setDraft(provider);
    const switched = await run(
      () => callCommand<AppConfig>("switch_provider", { id: provider.id }),
      "Current provider switched",
    );
    setConfig(switched);
    if (status?.running) {
      await callCommand<void>("write_codex_takeover");
    }
    await load();
  }

  async function openCodexHome() {
    await runAccount(() => callCommand<void>("open_codex_home"), "Codex home opened");
  }

  function openAdvancedDetails() {
    const details = document.querySelector<HTMLDetailsElement>(".advanced-row");
    if (!details) return;
    details.open = true;
    details.scrollIntoView({ behavior: "smooth", block: "center" });
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

  return (
    <main className="app-shell">
      <section className="tray-window" aria-label="CodexPilot">
        <header className="pilot-header">
          <div className="brand-lockup">
            <div className="brand-mark">CP</div>
            <div className="brand-copy">
              <h1>CodexPilot</h1>
              <p>Provider control</p>
            </div>
          </div>
          <button
            className="header-refresh"
            disabled={busy}
            onClick={() => void load()}
            title="Refresh"
            type="button"
          >
            <RefreshCw size={28} />
          </button>
        </header>

        <section className="hero-card">
          <div className="hero-provider">
            <div className={`hero-logo ${providerKind(currentProvider?.name ?? "")}`}>
              {providerDisplayIcon(currentProvider?.name ?? "P")}
              {currentProvider ? <span /> : null}
            </div>
            <div>
              <p>Active provider</p>
              <h2>{currentProvider?.name ?? "No provider"}</h2>
              <div className={`run-pill ${status?.running ? "on" : ""}`}>
                <span />
                {status?.running ? "Running" : "Stopped"}
              </div>
            </div>
          </div>

          <div className="hero-usage">
            <p>Account usage</p>
            {activeAccount ? (
              <>
                <div className="usage-metrics">
                  <span>Session</span>
                  <strong>{formatUsagePercent(activeUsage?.sessionPercent ?? null)}</strong>
                  <i />
                  <span>Weekly</span>
                  <strong>{formatUsagePercent(activeUsage?.weeklyPercent ?? null)}</strong>
                </div>
                <div className="hero-bars" aria-hidden="true">
                  <i
                    style={
                      {
                        "--usage-fill": usageFill(activeUsage?.sessionPercent ?? null),
                      } as React.CSSProperties
                    }
                  />
                  <i
                    style={
                      {
                        "--usage-fill": usageFill(activeUsage?.weeklyPercent ?? null),
                      } as React.CSSProperties
                    }
                  />
                </div>
              </>
            ) : (
              <p className="hero-empty">No Codex account detected</p>
            )}
          </div>

          {status?.running ? (
            <button className="hero-stop" disabled={busy} onClick={stopProxy} type="button">
              <span />
              Stop
            </button>
          ) : (
            <button className="hero-stop" disabled={busy} onClick={startProxy} type="button">
              <Power size={22} />
              Start
            </button>
          )}
        </section>

        <section className="providers-block">
          <div className="block-heading">
            <h3>Providers</h3>
            <button disabled={busy} onClick={addProvider} title="Add provider" type="button">
              <Plus size={24} />
            </button>
          </div>
          {config?.providers.length ? (
            <div className="provider-table">
              {config.providers.map((provider) => {
                const active = provider.id === config.currentProviderId;
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
                      <strong>{provider.name || "Unnamed provider"}</strong>
                      <small>{provider.model || provider.baseUrl}</small>
                    </span>
                    <span className="provider-state">
                      {active ? <CheckCircle2 size={26} /> : <span>Use</span>}
                    </span>
                  </button>
                );
              })}
            </div>
          ) : (
            <div className="empty-panel">
              <strong>No providers configured</strong>
              <span>Add a provider to start the local Codex proxy.</span>
              <button disabled={busy} onClick={addProvider} type="button">
                <Plus size={18} />
                Add provider
              </button>
            </div>
          )}
        </section>

        <section className="route-block">
          <h3>Current route</h3>
          <div className="route-card">
            <div>
              <Box size={26} />
              <span>Model</span>
              <strong>{draft?.model ?? currentProvider?.model ?? "No model"}</strong>
            </div>
            <div>
              <ArrowLeftRight size={26} />
              <span>Mode</span>
              <strong>
                {draft ?? currentProvider
                  ? (draft ?? currentProvider)?.apiFormat === "open_ai_chat"
                    ? "Chat transform"
                    : "Responses direct"
                  : "No format"}
              </strong>
            </div>
            <div>
              <Shield size={26} />
              <span>Sessions</span>
              <strong>Preserved</strong>
            </div>
          </div>

          <details className="advanced-row">
            <summary>
              <span>
                <Settings size={29} />
                <span>
                  <strong>Advanced details</strong>
                  <small>Local proxy · History mapping · Backups</small>
                </span>
              </span>
              <ChevronDown size={24} />
            </summary>
            <div className="advanced-content">
              <label>
                Provider name
                <input
                  value={draft?.name ?? ""}
                  placeholder="DeepSeek, OpenAI, Custom..."
                  onChange={(event) =>
                    draft && setDraft({ ...draft, name: event.currentTarget.value })
                  }
                />
              </label>
              <label>
                Base URL
                <input
                  value={draft?.baseUrl ?? ""}
                  placeholder="https://api.example.com/v1"
                  onChange={(event) =>
                    draft && setDraft({ ...draft, baseUrl: event.currentTarget.value })
                  }
                />
              </label>
              <label>
                API Key
                <span className="input-icon">
                  <KeyRound size={17} />
                  <input
                    type="password"
                    value={draft?.apiKey ?? ""}
                    placeholder="API key"
                    onChange={(event) =>
                      draft && setDraft({ ...draft, apiKey: event.currentTarget.value })
                    }
                  />
                </span>
              </label>
              <label>
                Model
                <input
                  value={draft?.model ?? ""}
                  placeholder="model name"
                  onChange={(event) =>
                    draft && setDraft({ ...draft, model: event.currentTarget.value })
                  }
                />
              </label>
              <label>
                API format
                <select
                  value={draft?.apiFormat ?? "open_ai_responses"}
                  onChange={(event) =>
                    draft &&
                    setDraft({ ...draft, apiFormat: event.currentTarget.value as ApiFormat })
                  }
                >
                  <option value="open_ai_responses">Responses direct</option>
                  <option value="open_ai_chat">Chat transform</option>
                </select>
              </label>
              <span className="proxy-line">Local proxy: {routeUrl}</span>
              <div className="advanced-actions">
                <button className="primary-button" disabled={busy || !draft} onClick={saveDraft} type="button">
                  <Save size={16} />
                  Save
                </button>
                <button className="soft-button" disabled={busy || !draft} onClick={switchToSelected} type="button">
                  Use provider
                </button>
                <button
                  className="danger-button"
                  disabled={
                    busy ||
                    !draft ||
                    (draftIsSaved && (config?.providers.length ?? 0) <= 1)
                  }
                  onClick={deleteSelected}
                  title={draftIsSaved ? "Delete provider" : "Discard draft"}
                  type="button"
                >
                  <Trash2 size={16} />
                </button>
              </div>
            </div>
          </details>
          {message && <p className="message">{message}</p>}
        </section>

        <section className="accounts-block">
          <div className="block-heading">
            <h3>Accounts</h3>
            <button disabled={busy} onClick={addManagedAccount} title="Add login" type="button">
              <Plus size={24} />
            </button>
          </div>

          {accounts?.hasUnreadableStore && (
            <p className="message">Managed account store is unreadable.</p>
          )}

          <div className="account-table">
            {accounts?.accounts.length ? (
              accounts.accounts.map((account) => {
                const usage = accountUsage[account.id];
                return (
                  <div className="account-row" key={account.id}>
                    <span className={`account-avatar ${account.isActive ? "active" : ""}`}>
                      {accountInitial(account.email)}
                    </span>
                    <span className="account-copy">
                      <strong>
                        {account.email}
                        {account.isActive ? <em>Active</em> : null}
                      </strong>
                      <small>{account.isLive ? "Live system account" : "Managed home"}</small>
                    </span>
                    <span className="account-meter">
                      <span>Session {formatUsagePercent(usage?.sessionPercent ?? null)}</span>
                      <i
                        style={
                          {
                            "--usage-fill": usageFill(usage?.sessionPercent ?? null),
                          } as React.CSSProperties
                        }
                      />
                    </span>
                    <span className="account-meter">
                      <span>Weekly {formatUsagePercent(usage?.weeklyPercent ?? null)}</span>
                      <i
                        style={
                          {
                            "--usage-fill": usageFill(usage?.weeklyPercent ?? null),
                          } as React.CSSProperties
                        }
                      />
                    </span>
                    <span className="account-menu">
                      {!account.isActive ? (
                        <button disabled={busy} onClick={() => switchAccount(account.id)} type="button">
                          Switch
                        </button>
                      ) : null}
                      <button
                        disabled={busy}
                        onClick={() =>
                          account.isLive ? importCurrentAccount() : refreshManagedAccount(account.id)
                        }
                        title={account.isLive ? "Import current" : "Refresh account"}
                        type="button"
                      >
                        <MoreHorizontal size={24} />
                      </button>
                    </span>
                  </div>
                );
              })
            ) : (
              <p className="empty">No Codex accounts detected.</p>
            )}
          </div>
          {accountMessage && <p className="message">{accountMessage}</p>}
        </section>

        <footer className="pilot-footer">
          <button onClick={() => void openCodexHome()} title="Open Codex Home" type="button">
            <Folder size={25} />
            <span>Open Codex Home</span>
          </button>
          <button onClick={openAdvancedDetails} title="Settings" type="button">
            <Settings size={25} />
            <span>Settings</span>
          </button>
          <button onClick={() => void openCodexHome()} type="button" title="Open Codex Home">
            <ExternalLink size={25} />
          </button>
        </footer>
      </section>
    </main>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
