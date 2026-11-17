import type {
  CacheStatsDto,
  ChatMessagePayload,
  CreateKeyInput,
  CreateKeyResponse,
  CreateProviderInput,
  CreateProviderKeyInput,
  CreateUserInput,
  GuardrailDto,
  GuardrailInput,
  LogsQuery,
  LogsResponse,
  OverviewResponse,
  OverviewWindow,
  PricingDto,
  CreateMcpServerInput,
  McpServerDto,
  McpToolEntryDto,
  PromptCreateInput,
  PromptRenderResponse,
  PromptSummaryDto,
  PromptVersionDto,
  PromptVersionsResponse,
  SemanticStatsDto,
  UpdateMcpServerInput,
  UpdatePromptInput,
  ProviderDto,
  ProviderKeyDto,
  ProviderTestResult,
  RequestLogDto,
  Route,
  RouteInput,
  RoutingPolicy,
  SettingsDto,
  StreamStats,
  SyncModelsResult,
  TeamDto,
  UpdateGuardrailInput,
  UpdateKeyInput,
  UpdateProviderInput,
  UpdateProviderKeyInput,
  UpdateUserInput,
  UpdateWebhookInput,
  UserDto,
  VirtualKeyDto,
  WebhookDto,
  WebhookInput,
  WebhookTestResult,
} from './types';
import { apiFetch } from './auth';

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await apiFetch(path, {
    ...init,
    headers: {
      'Content-Type': 'application/json',
      ...(init?.headers ?? {}),
    },
  });

  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    try {
      const body = (await res.json()) as { error?: { message?: string } };
      if (body?.error?.message) {
        message = body.error.message;
      }
    } catch {
      // ignore non-JSON error body
    }
    throw new ApiError(res.status, message);
  }

  return (await res.json()) as T;
}

export const api = {
  getOverview: (window: OverviewWindow) =>
    request<OverviewResponse>(`/admin/api/overview?window=${encodeURIComponent(window)}`),

  listProviders: async (): Promise<ProviderDto[]> =>
    (await request<{ providers: ProviderDto[] }>('/admin/api/providers')).providers,

  createProvider: (body: CreateProviderInput) =>
    request<ProviderDto>('/admin/api/providers', {
      method: 'POST',
      body: JSON.stringify(body),
    }),

  updateProvider: (id: string, body: UpdateProviderInput) =>
    request<ProviderDto>(`/admin/api/providers/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(body),
    }),

  deleteProvider: (id: string) =>
    request<{ deleted: boolean; id: string }>(`/admin/api/providers/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),

  listProviderKeys: async (providerId: string): Promise<ProviderKeyDto[]> =>
    (await request<{ keys: ProviderKeyDto[] }>(`/admin/api/providers/${encodeURIComponent(providerId)}/keys`))
      .keys,

  createProviderKey: (providerId: string, body: CreateProviderKeyInput) =>
    request<{ key: ProviderKeyDto }>(`/admin/api/providers/${encodeURIComponent(providerId)}/keys`, {
      method: 'POST',
      body: JSON.stringify(body),
    }),

  updateProviderKey: (providerId: string, keyId: string, body: UpdateProviderKeyInput) =>
    request<{ key: ProviderKeyDto }>(
      `/admin/api/providers/${encodeURIComponent(providerId)}/keys/${encodeURIComponent(keyId)}`,
      { method: 'PATCH', body: JSON.stringify(body) },
    ),

  deleteProviderKey: (providerId: string, keyId: string) =>
    request<{ deleted: boolean; id: string }>(
      `/admin/api/providers/${encodeURIComponent(providerId)}/keys/${encodeURIComponent(keyId)}`,
      { method: 'DELETE' },
    ),

  testProvider: (id: string) =>
    request<ProviderTestResult>(`/admin/api/providers/${encodeURIComponent(id)}/test`, {
      method: 'POST',
    }),

  syncProviderModels: (id: string) =>
    request<SyncModelsResult>(`/admin/api/providers/${encodeURIComponent(id)}/sync-models`, {
      method: 'POST',
    }),

  getRoutingPolicy: async (): Promise<RoutingPolicy> =>
    (await request<{ policy: RoutingPolicy }>('/admin/api/routing/policy')).policy,

  saveRoutingPolicy: (policy: RoutingPolicy) =>
    request<{ policy: RoutingPolicy }>('/admin/api/routing/policy', {
      method: 'PUT',
      body: JSON.stringify(policy),
    }),

  listRoutes: async (): Promise<Route[]> =>
    (await request<{ routes: Route[] }>('/admin/api/routes')).routes,

  getRoute: (alias: string) => request<Route>(`/admin/api/routes/${encodeURIComponent(alias)}`),

  putRoute: (alias: string, body: RouteInput) =>
    request<Route>(`/admin/api/routes/${encodeURIComponent(alias)}`, {
      method: 'PUT',
      body: JSON.stringify(body),
    }),

  deleteRoute: (alias: string) =>
    request<unknown>(`/admin/api/routes/${encodeURIComponent(alias)}`, {
      method: 'DELETE',
    }),

  /**
   * Mints the one key the console playground reuses for its session.
   *
   * No allow-list: an empty list means "any model", so the operator can switch
   * aliases without the key rejecting the request.
   */
  createPlaygroundKey: async (): Promise<CreateKeyResponse> => {
    const expiresAt = new Date(Date.now() + 12 * 60 * 60 * 1000).toISOString();
    return request<CreateKeyResponse>('/admin/api/keys', {
      method: 'POST',
      body: JSON.stringify({
        name: `playground ${new Date().toISOString().slice(0, 16)}`,
        allowed_models: [],
        budget_usd: 5,
        rpm_limit: 60,
        tpm_limit: 20000,
        expires_at: expiresAt,
        tags: ['playground'],
      }),
    });
  },

  listKeys: async (): Promise<VirtualKeyDto[]> =>
    (await request<{ keys: VirtualKeyDto[] }>('/admin/api/keys')).keys,

  createKey: (body: CreateKeyInput) =>
    request<CreateKeyResponse>('/admin/api/keys', {
      method: 'POST',
      body: JSON.stringify(body),
    }),

  updateKey: (id: string, body: UpdateKeyInput) =>
    request<VirtualKeyDto>(`/admin/api/keys/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(body),
    }),

  deleteKey: (id: string) =>
    request<{ deleted: boolean; id: string }>(`/admin/api/keys/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),

  listTeams: async (): Promise<TeamDto[]> =>
    (await request<{ teams: TeamDto[] }>('/admin/api/teams')).teams,

  listGuardrails: async (): Promise<GuardrailDto[]> =>
    (await request<{ guardrails: GuardrailDto[] }>('/admin/api/guardrails')).guardrails,

  createGuardrail: async (body: GuardrailInput): Promise<GuardrailDto> =>
    (await request<{ guardrail: GuardrailDto }>('/admin/api/guardrails', {
      method: 'POST',
      body: JSON.stringify(body),
    })).guardrail,

  updateGuardrail: async (id: string, body: UpdateGuardrailInput): Promise<GuardrailDto> =>
    (await request<{ guardrail: GuardrailDto }>(`/admin/api/guardrails/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(body),
    })).guardrail,

  deleteGuardrail: (id: string) =>
    request<{ deleted: boolean }>(`/admin/api/guardrails/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),

  listWebhooks: async (): Promise<WebhookDto[]> =>
    (await request<{ webhooks: WebhookDto[] }>('/admin/api/webhooks')).webhooks,

  createWebhook: async (body: WebhookInput): Promise<WebhookDto> =>
    (await request<{ webhook: WebhookDto }>('/admin/api/webhooks', {
      method: 'POST',
      body: JSON.stringify(body),
    })).webhook,

  updateWebhook: async (id: string, body: UpdateWebhookInput): Promise<WebhookDto> =>
    (await request<{ webhook: WebhookDto }>(`/admin/api/webhooks/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(body),
    })).webhook,

  deleteWebhook: (id: string) =>
    request<{ deleted: boolean }>(`/admin/api/webhooks/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),

  testWebhook: (id: string) =>
    request<WebhookTestResult>(`/admin/api/webhooks/${encodeURIComponent(id)}/test`, {
      method: 'POST',
    }),

  getCacheStats: () => request<CacheStatsDto>('/admin/api/response-cache/stats'),

  clearCache: () =>
    request<{ cleared: boolean }>('/admin/api/response-cache', { method: 'DELETE' }),

  listLogs: (query: LogsQuery) => {
    const params = new URLSearchParams();
    if (query.page) params.set('page', String(query.page));
    if (query.page_size) params.set('page_size', String(query.page_size));
    if (query.status) params.set('status', query.status);
    if (query.model) params.set('model', query.model);
    if (query.key_id) params.set('key_id', query.key_id);
    if (query.search) params.set('search', query.search);
    return request<LogsResponse>(`/admin/api/logs?${params.toString()}`);
  },

  getLog: (id: string) =>
    request<RequestLogDto>(`/admin/api/logs/${encodeURIComponent(id)}`),

  listPricing: async (): Promise<PricingDto[]> =>
    (await request<{ pricing: PricingDto[] }>('/admin/api/pricing')).pricing,

  getSettings: () => request<SettingsDto>('/admin/api/settings'),

  putSettings: (body: SettingsDto) =>
    request<SettingsDto>('/admin/api/settings', {
      method: 'PUT',
      body: JSON.stringify(body),
    }),

  listPrompts: async (): Promise<PromptSummaryDto[]> =>
    (await request<{ prompts: PromptSummaryDto[] }>('/admin/api/prompts')).prompts,

  listPromptVersions: (slug: string) =>
    request<PromptVersionsResponse>(`/admin/api/prompts/${encodeURIComponent(slug)}`),

  createPrompt: (body: PromptCreateInput) =>
    request<{ prompt: PromptVersionDto }>('/admin/api/prompts', {
      method: 'POST',
      body: JSON.stringify(body),
    }),

  updatePrompt: (slug: string, version: number, body: UpdatePromptInput) =>
    request<{ prompt: PromptVersionDto }>(
      `/admin/api/prompts/${encodeURIComponent(slug)}/${version}`,
      { method: 'PATCH', body: JSON.stringify(body) },
    ),

  deletePrompt: (slug: string) =>
    request<{ deleted: boolean; versions: number }>(
      `/admin/api/prompts/${encodeURIComponent(slug)}`,
      { method: 'DELETE' },
    ),

  activatePrompt: (slug: string, version: number) =>
    request<{ prompt: PromptVersionDto }>(
      `/admin/api/prompts/${encodeURIComponent(slug)}/activate/${version}`,
      { method: 'POST' },
    ),

  renderPrompt: (slug: string, variables: Record<string, string>, version?: number) =>
    request<PromptRenderResponse>(`/admin/api/prompts/${encodeURIComponent(slug)}/render`, {
      method: 'POST',
      body: JSON.stringify(version ? { variables, version } : { variables }),
    }),

  getSemanticStats: () => request<SemanticStatsDto>('/admin/api/semantic/stats'),

  putSemanticThreshold: (threshold: number) =>
    request<SemanticStatsDto>('/admin/api/semantic/threshold', {
      method: 'PUT',
      body: JSON.stringify({ threshold }),
    }),

  putSemanticEmbeddingModel: (embedding_model: string | null) =>
    request<SemanticStatsDto>('/admin/api/semantic/embedding-model', {
      method: 'PUT',
      body: JSON.stringify({ embedding_model }),
    }),

  clearSemanticCache: () =>
    request<{ cleared: boolean }>('/admin/api/semantic-cache', { method: 'DELETE' }),

  listMcpServers: async (): Promise<McpServerDto[]> =>
    (await request<{ servers: McpServerDto[] }>('/admin/api/mcp/servers')).servers,

  createMcpServer: (body: CreateMcpServerInput) =>
    request<{ server: McpServerDto }>('/admin/api/mcp/servers', {
      method: 'POST',
      body: JSON.stringify(body),
    }),

  updateMcpServer: (id: string, body: UpdateMcpServerInput) =>
    request<{ server: McpServerDto }>(`/admin/api/mcp/servers/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(body),
    }),

  deleteMcpServer: (id: string) =>
    request<{ deleted: boolean }>(`/admin/api/mcp/servers/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),

  refreshMcpServer: (id: string) =>
    request<{ server: McpServerDto; discovered: number }>(
      `/admin/api/mcp/servers/${encodeURIComponent(id)}/refresh`,
      { method: 'POST' },
    ),

  listMcpTools: async (): Promise<McpToolEntryDto[]> =>
    (await request<{ tools: McpToolEntryDto[] }>('/admin/api/mcp/tools')).tools,

  listUsers: async (): Promise<UserDto[]> =>
    (await request<{ users: UserDto[] }>('/admin/api/users')).users,

  createUser: async (body: CreateUserInput): Promise<UserDto> =>
    (await request<{ user: UserDto }>('/admin/api/users', {
      method: 'POST',
      body: JSON.stringify(body),
    })).user,

  updateUser: (id: string, body: UpdateUserInput) =>
    request<UserDto>(`/admin/api/users/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(body),
    }),

  deleteUser: (id: string) =>
    request<{ deleted: boolean; id: string }>(`/admin/api/users/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),

  resetUserPassword: (id: string, password: string) =>
    request<{ updated: boolean; id: string }>(`/admin/api/users/${encodeURIComponent(id)}/password`, {
      method: 'POST',
      body: JSON.stringify({ password }),
    }),

  changeOwnPassword: (current: string, next: string) =>
    request<{ updated: boolean }>('/admin/api/users/me/password', {
      method: 'POST',
      body: JSON.stringify({ current_password: current, new_password: next }),
    }),
};

export interface StreamChatOptions {
  apiKey: string;
  model: string;
  messages: ChatMessagePayload[];
  temperature: number;
  maxTokens: number;
  signal: AbortSignal;
  template?: string;
  templateVersion?: number;
  templateVariables?: Record<string, string>;
  onDelta: (text: string) => void;
  onStats: (stats: StreamStats) => void;
}

export async function streamChatCompletion(opts: StreamChatOptions): Promise<string> {
  const startedAt = performance.now();
  const res = await fetch('/v1/chat/completions', {
    method: 'POST',
    signal: opts.signal,
    // The proxy authenticates on the bearer key alone. Sending the operator's
    // admin session cookie here would leak console credentials onto LLM traffic.
    credentials: 'omit',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${opts.apiKey.trim()}`,
    },
    body: JSON.stringify({
      model: opts.model,
      messages: opts.messages,
      temperature: opts.temperature,
      max_tokens: opts.maxTokens,
      stream: true,
      ...(opts.template
        ? {
            template: opts.template,
            ...(opts.templateVersion ? { template_version: opts.templateVersion } : {}),
            template_variables: opts.templateVariables ?? {},
          }
        : {}),
    }),
  });

  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    try {
      const body = (await res.json()) as { error?: { message?: string } };
      if (body?.error?.message) {
        message = body.error.message;
      }
    } catch {
      // Non-JSON error body; keep the status line.
    }
    throw new ApiError(res.status, message);
  }

  if (!res.body) {
    throw new Error('Response body is missing');
  }

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  let fullText = '';
  let ttftMs: number | null = null;
  let firstTokenAt: number | null = null;
  let chunkCount = 0;
  let promptTokens = 0;
  let outputTokens = 0;

  const emitStats = () => {
    const now = performance.now();
    const totalMs = now - startedAt;
    const decodeSec = Math.max(((firstTokenAt ?? now) ? now - (firstTokenAt ?? startedAt) : totalMs) / 1000, 0.001);
    const estTokens = outputTokens > 0 ? outputTokens : chunkCount;
    opts.onStats({
      ttftMs,
      totalMs,
      outputTokens: estTokens,
      promptTokens,
      tokensPerSec: estTokens / decodeSec,
    });
  };

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });

    const lines = buffer.split(/\r?\n/);
    buffer = lines.pop() ?? '';

    for (const rawLine of lines) {
      const line = rawLine.trim();
      if (!line.startsWith('data:')) continue;
      const payload = line.slice(5).trim();
      if (!payload || payload === '[DONE]') continue;

      try {
        const parsed = JSON.parse(payload) as {
          choices?: Array<{ delta?: { content?: string } }>;
          usage?: { prompt_tokens?: number; completion_tokens?: number };
        };

        const delta = parsed.choices?.[0]?.delta?.content;
        if (typeof delta === 'string' && delta.length > 0) {
          if (ttftMs === null) {
            firstTokenAt = performance.now();
            ttftMs = firstTokenAt - startedAt;
          }
          chunkCount += 1;
          fullText += delta;
          opts.onDelta(fullText);
        }

        if (parsed.usage) {
          if (typeof parsed.usage.prompt_tokens === 'number') {
            promptTokens = parsed.usage.prompt_tokens;
          }
          if (typeof parsed.usage.completion_tokens === 'number') {
            outputTokens = parsed.usage.completion_tokens;
          }
        }
        emitStats();
      } catch {
        // ignore malformed SSE keep-alive/partial comment lines
      }
    }
  }

  emitStats();
  return fullText;
}
