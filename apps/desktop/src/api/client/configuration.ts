import type {
  AgentRuntimeSettings,
  AppSettings,
  CodexAccountStatus,
  CodexLoginStart,
  ExperienceMode,
  LibraryIngestionResult,
  LibraryProviderDescriptor,
  LibraryProviderId,
  LibraryProviderStatus,
  LibrarySearchRequest,
  LibrarySearchResponse,
  LibrarySourcePage,
  PermissionMode,
  ProviderDriverDescriptor,
  ProviderHealth,
  ProviderKind,
  ProviderSettings,
  SagIngestionResult,
  SagLibraryStatus,
  SagSearchRequest,
  SagSearchResponse,
  SagSource,
  SkillDescriptor,
  WindowsSandboxSetupStatus,
} from "../../types";
import { ApiTransport, parseResponse, queryString } from "./transport";

export class ConfigurationApi extends ApiTransport {
  async health(): Promise<{
    ok: boolean;
    service: string;
    apiVersion: number;
    shellRuntime: {
      runtime: {
        program: string;
        dialect: "power_shell7" | "windows_power_shell51" | "posix_sh";
        version: string | null;
        source:
          "configured" | "managed" | "standard_install" | "path" | "system";
      };
      managedVersion: string;
      managedStatus:
        | "not_required"
        | "pending"
        | "downloading"
        | "ready"
        | "disabled"
        | "failed";
      managedError?: string;
    };
  }> {
    return this.get("health", "/health");
  }

  async getSettings(): Promise<AppSettings> {
    return this.get("getSettings", "/api/settings");
  }

  async updateSettings(input: {
    providers?: ProviderSettings[];
    activeProviderId?: string;
    providerKind?: ProviderKind;
    baseUrl?: string;
    model?: string;
    apiKeySource?: string;
    permissionMode?: PermissionMode;
    agentRuntime?: AgentRuntimeSettings;
    defaultWorkspaceRoot?: string;
    clearDefaultWorkspaceRoot?: boolean;
    sandbox?: AppSettings["sandbox"];
  }): Promise<AppSettings> {
    return this.patch("updateSettings", "/api/settings", input);
  }

  async getWindowsSandboxSetup(): Promise<WindowsSandboxSetupStatus> {
    return this.get("getWindowsSandboxSetup", "/api/sandbox/windows/setup");
  }

  async getSagLibraryStatus(signal?: AbortSignal): Promise<SagLibraryStatus> {
    return this.getLibraryProviderStatus(
      "sag",
      signal,
    ) as Promise<SagLibraryStatus>;
  }

  async listSagSources(signal?: AbortSignal): Promise<SagSource[]> {
    const page = await this.listLibrarySources("sag", {}, signal);
    return page.items as SagSource[];
  }

  async searchSag(input: SagSearchRequest): Promise<SagSearchResponse> {
    return this.searchLibrary("sag", input) as Promise<SagSearchResponse>;
  }

  async listLibraryProviders(
    signal?: AbortSignal,
  ): Promise<LibraryProviderDescriptor[]> {
    return this.get("listLibraryProviders", "/api/library/providers", signal);
  }

  async getLibraryProviderStatus(
    provider: LibraryProviderId,
    signal?: AbortSignal,
  ): Promise<LibraryProviderStatus> {
    return this.get(
      "getLibraryProviderStatus",
      `/api/library/${provider}/status`,
      signal,
    );
  }

  async listLibrarySources(
    provider: LibraryProviderId,
    options: { query?: string; offset?: number; limit?: number } = {},
    signal?: AbortSignal,
  ): Promise<LibrarySourcePage> {
    const params = new URLSearchParams({
      offset: String(options.offset ?? 0),
      limit: String(options.limit ?? 100),
    });
    const query = options.query?.trim();
    if (query) params.set("query", query);
    return this.get(
      "listLibrarySources",
      `/api/library/${provider}/sources?${params.toString()}`,
      signal,
    );
  }

  async searchLibrary(
    provider: LibraryProviderId,
    input: LibrarySearchRequest,
  ): Promise<LibrarySearchResponse> {
    return this.post("searchLibrary", `/api/library/${provider}/search`, input);
  }

  async ingestSagText(input: {
    content: string;
    filename?: string;
    assetId?: string;
    sourceKey?: string;
    namespace?: string;
    title?: string;
    metadata?: Record<string, unknown>;
  }): Promise<SagIngestionResult> {
    return this.post(
      "ingestSagText",
      "/api/library/sag/ingestions/text",
      input,
    );
  }

  async uploadSagSource(input: {
    file: File;
    assetId?: string;
    sourceKey?: string;
    namespace?: string;
    title?: string;
    metadata?: Record<string, unknown>;
  }): Promise<SagIngestionResult> {
    return this.uploadLibrarySource(
      "sag",
      input,
    ) as Promise<SagIngestionResult>;
  }

  async uploadLibrarySource(
    provider: LibraryProviderId,
    input: {
      file: File;
      assetId?: string;
      sourceKey?: string;
      namespace?: string;
      title?: string;
      metadata?: Record<string, unknown>;
    },
  ): Promise<LibraryIngestionResult> {
    const form = new FormData();
    form.set("file", input.file, input.file.name);
    if (input.assetId) form.set("asset_id", input.assetId);
    if (input.sourceKey) form.set("source_key", input.sourceKey);
    form.set("namespace", input.namespace || "enterprise_knowledge");
    if (input.title) form.set("title", input.title);
    form.set("metadata_json", JSON.stringify(input.metadata ?? {}));
    const response = await fetch(
      `${this.baseUrl}/api/library/${provider}/ingestions/upload`,
      {
        method: "POST",
        headers: this.authHeaders(),
        body: form,
      },
    );
    return parseResponse<LibraryIngestionResult>(
      response,
      "uploadLibrarySource",
    );
  }

  async setupWindowsSandbox(): Promise<WindowsSandboxSetupStatus> {
    return this.post("setupWindowsSandbox", "/api/sandbox/windows/setup", {});
  }

  async removeWindowsSandbox(): Promise<WindowsSandboxSetupStatus> {
    return this.delete("removeWindowsSandbox", "/api/sandbox/windows/setup");
  }

  async getProviderHealth(): Promise<ProviderHealth[]> {
    return this.get("getProviderHealth", "/api/provider/health");
  }

  async listProviderDrivers(): Promise<ProviderDriverDescriptor[]> {
    return this.get("listProviderDrivers", "/api/provider/drivers");
  }

  async getCodexAccount(): Promise<CodexAccountStatus> {
    return this.get("getCodexAccount", "/api/codex/account");
  }

  async startCodexLogin(deviceCode = false): Promise<CodexLoginStart> {
    return this.post("startCodexLogin", "/api/codex/account/login", {
      deviceCode,
    });
  }

  async cancelCodexLogin(): Promise<void> {
    await this.post("cancelCodexLogin", "/api/codex/account/login/cancel", {});
  }

  async logoutCodexAccount(): Promise<void> {
    await this.post("logoutCodexAccount", "/api/codex/account/logout", {});
  }

  async listSkills(
    workspaceRoot?: string | null,
    threadId?: string | null,
    experienceMode?: ExperienceMode,
  ): Promise<SkillDescriptor[]> {
    return this.get(
      "listSkills",
      `/api/skills${queryString({
        workspaceRoot: workspaceRoot ?? undefined,
        threadId: threadId ?? undefined,
        experienceMode,
      })}`,
    );
  }
}
