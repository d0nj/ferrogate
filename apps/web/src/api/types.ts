export type OverviewWindow = '1h' | '24h' | '7d';

export interface OverviewKpis {
  total_requests: number;
  rpm: number;
  total_spend_usd: number;
  ttft_p50_ms: number | null;
  ttft_p95_ms: number | null;
  error_rate_pct: number;
  fallback_rate_pct: number;
  cache_hit_rate_pct: number;
}

export interface ModelBucketMetric {
  model: string;
  provider: string;
  tokens: number;
  spend_usd: number;
}

export interface TimeseriesBucket {
  bucket: string;
  label: string;
  requests: number;
  input_tokens: number;
  output_tokens: number;
  spend_usd: number;
  by_model: ModelBucketMetric[];
}

export interface ProviderHealth {
  provider_id: string;
  name: string;
  provider_type: string;
  is_active: boolean;
  circuit_state: 'closed' | 'open' | 'half_open' | string;
  consecutive_failures: number;
  total_requests: number;
  total_failures: number;
  last_error: string | null;
}

export interface OverviewResponse {
  window: OverviewWindow;
  kpis: OverviewKpis;
  timeseries: TimeseriesBucket[];
  providers_health: ProviderHealth[];
}

export type ProviderKind = 'openai' | 'anthropic' | 'gemini' | 'typesafe';

export type ApiDialect = 'chat' | 'responses';

export interface ProviderDto {
  id: string;
  name: string;
  provider_type: ProviderKind | string;
  api_dialect: ApiDialect | string;
  base_url: string;
  api_key_masked: string;
  has_api_key: boolean;
  pool_key_count: number;
  models: string[];
  is_active: boolean;
  timeout_ms: number;
  created_at: string;
  updated_at: string;
}

export interface CreateProviderInput {
  name: string;
  provider_type: ProviderKind;
  api_dialect?: ApiDialect;
  base_url: string;
  api_key: string;
  models: string[];
  is_active?: boolean;
  timeout_ms?: number;
}

export interface UpdateProviderInput {
  name?: string;
  provider_type?: ProviderKind;
  api_dialect?: ApiDialect;
  base_url?: string;
  api_key?: string;
  clear_api_key?: boolean;
  models?: string[];
  is_active?: boolean;
  timeout_ms?: number;
}

/** One ordered target of an alias; array order is the attempt order. */
export interface RouteTarget {
  provider_id: string;
  provider_name: string;
  upstream_model: string;
  weight: number;
}

export interface Route {
  alias: string;
  strategy: TierStrategy | null;
  fallbacks: string[];
  targets: RouteTarget[];
}

export interface RouteTargetInput {
  provider_id: string;
  upstream_model: string;
  weight: number;
}

export interface RouteInput {
  alias: string;
  strategy: TierStrategy | null;
  fallbacks: string[];
  targets: RouteTargetInput[];
}

export interface VirtualKeyDto {
  id: string;
  name: string;
  key_prefix: string;
  budget_usd: number | null;
  spent_usd: number;
  rpm_limit: number | null;
  tpm_limit: number | null;
  allowed_models: string[];
  allowed_mcp: string[];
  tags: string[];
  team_name: string | null;
  expires_at: string | null;
  is_active: boolean;
  created_at: string;
  last_used_at: string | null;
}

export type TierStrategy = 'weighted' | 'latency' | 'cost' | 'priority'

export interface RetryClasses {
  rate_limit: boolean
  server_error: boolean
  transport: boolean
  protocol: boolean
}

export interface TierStrategyOverride {
  position: number
  strategy: TierStrategy
}

export interface RoutingPolicy {
  failure_threshold: number
  cooldown_seconds: number
  cache_ttl_seconds: number
  key_rotations: number
  strategy: TierStrategy
  tier_strategies: TierStrategyOverride[]
  retry_on: RetryClasses
  max_attempts: number
}

export interface CreateKeyInput {
  name: string;
  budget_usd?: number | null;
  rpm_limit?: number | null;
  tpm_limit?: number | null;
  allowed_models?: string[];
  expires_at?: string;
  team_name?: string | null;
  tags?: string[];
  allowed_mcp?: string[];
}

export interface CreateKeyResponse {
  key: VirtualKeyDto;
  plaintext_key: string;
}

export interface UpdateKeyInput {
  name?: string;
  budget_usd?: number;
  clear_budget?: boolean;
  rpm_limit?: number;
  clear_rpm_limit?: boolean;
  tpm_limit?: number;
  clear_tpm_limit?: boolean;
  allowed_models?: string[];
  is_active?: boolean;
  reset_spend?: boolean;
  expires_at?: string;
  clear_expires_at?: boolean;
  team_name?: string | null;
  tags?: string[];
  allowed_mcp?: string[];
}

export interface RoutingAttempt {
  provider_id: string;
  provider_name: string;
  model: string;
  status_code: number;
  latency_ms: number;
  error: string | null;
}

export interface ChatMessagePayload {
  role: string;
  content: unknown;
}

export interface RequestLogDto {
  id: string;
  created_at: string;
  virtual_key_id: string | null;
  virtual_key_name: string | null;
  requested_model: string;
  routed_provider_id: string | null;
  routed_provider_name: string | null;
  routed_model: string | null;
  status_code: number;
  is_streaming: boolean;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  ttft_ms: number | null;
  latency_ms: number;
  fallback_count: number;
  cache_hit: boolean;
  routing_trace: RoutingAttempt[];
  request_messages: ChatMessagePayload[] | null;
  response_text: string | null;
  error_message: string | null;
}

export interface LogsQuery {
  page?: number;
  page_size?: number;
  status?: string;
  model?: string;
  key_id?: string;
  search?: string;
}

export interface LogsResponse {
  items: RequestLogDto[];
  total: number;
  page: number;
  page_size: number;
}

export interface PricingDto {
  model: string;
  provider_type: string;
  input_cost_per_1m: number;
  output_cost_per_1m: number;
  updated_at: string;
}

export interface SettingsDto {
  redact_prompts: boolean;
}

export interface StreamStats {
  ttftMs: number | null;
  totalMs: number;
  outputTokens: number;
  promptTokens: number;
  tokensPerSec: number;
}

export interface ProviderKeyDto {
  id: string;
  provider_id: string;
  name: string;
  api_key_masked: string;
  has_api_key: boolean;
  weight: number;
  models: string[];
  rpm_limit: number | null;
  is_active: boolean;
  in_cooldown: boolean;
  cooldown_until: string | null;
  last_error: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateProviderKeyInput {
  name: string;
  api_key: string;
  weight?: number;
  models?: string[];
  rpm_limit?: number | null;
  is_active?: boolean;
}

export interface UpdateProviderKeyInput {
  name?: string;
  api_key?: string;
  weight?: number;
  models?: string[];
  rpm_limit?: number | null;
  clear_rpm_limit?: boolean;
  is_active?: boolean;
  clear_cooldown?: boolean;
}

export interface ProviderTestResult {
  ok: boolean;
  status_code: number | null;
  latency_ms: number;
  models_found: number;
  error: string | null;
}

export interface SyncModelsResult {
  ok: boolean;
  added_count: number;
  /** Models the provider no longer lists; sync removes them from the stored list. */
  removed_count: number;
  removed: string[];
  models: string[];
}

export type GuardrailPhase = 'input' | 'output';
export type GuardrailRuleType = 'pii' | 'secret' | 'prompt_injection' | 'regex';
export type GuardrailAction = 'block' | 'redact' | 'flag';

export interface GuardrailDto {
  id: string;
  name: string;
  phase: GuardrailPhase | string;
  rule_type: GuardrailRuleType | string;
  pattern: string | null;
  action: GuardrailAction | string;
  priority: number;
  is_active: boolean;
  created_at: string;
  updated_at: string;
}

export interface GuardrailInput {
  name: string;
  phase: GuardrailPhase;
  rule_type: GuardrailRuleType;
  pattern?: string | null;
  action: GuardrailAction;
  priority?: number;
  is_active?: boolean;
}

export interface UpdateGuardrailInput {
  name?: string;
  phase?: GuardrailPhase;
  rule_type?: GuardrailRuleType;
  pattern?: string | null;
  action?: GuardrailAction;
  priority?: number;
  is_active?: boolean;
}

export interface WebhookDto {
  id: string;
  name: string;
  url: string;
  has_secret: boolean;
  secret_masked: string;
  events: string[];
  is_active: boolean;
  last_status: number | null;
  last_error: string | null;
  created_at: string;
  updated_at: string;
}

export interface WebhookInput {
  name: string;
  url: string;
  secret?: string | null;
  events: string[];
  is_active?: boolean;
}

export interface UpdateWebhookInput {
  name?: string;
  url?: string;
  secret?: string | null;
  events?: string[];
  is_active?: boolean;
}

export interface WebhookTestResult {
  delivered: boolean;
  status: number | null;
  error: string | null;
}

export interface TeamDto {
  name: string;
  keys: number;
  spend_usd: number;
}

export interface CacheStatsDto {
  entries: number;
  hits: number;
  misses: number;
  redis_connected: boolean;
  /** False when REDIS_URL was unset at startup: Redis cannot be enabled at runtime. */
  redis_configured: boolean;
}

export interface PromptVariable {
  name: string;
  default?: string | null;
}

export interface PromptSummaryDto {
  slug: string;
  name: string;
  latest_version: number;
  versions: number;
  is_active: boolean;
  model: string | null;
  temperature: number | null;
  max_tokens: number | null;
  variables: PromptVariable[];
  updated_at: string;
}

export interface PromptVersionDto {
  id: string;
  slug: string;
  version: number;
  name: string;
  system_prompt: string;
  user_template: string;
  model: string | null;
  temperature: number | null;
  max_tokens: number | null;
  variables: PromptVariable[];
  is_active: boolean;
  created_at: string;
  updated_at: string;
}

export interface PromptVersionsResponse {
  slug: string;
  versions: PromptVersionDto[];
}

export interface PromptRenderResponse {
  system: string;
  user: string;
  missing_variables: string[];
}

export interface SemanticStatsDto {
  entries: number;
  hits: number;
  threshold: number;
  backend: string;
  embedding_model: string | null;
  /** False when QDRANT_URL was unset at startup: vector search cannot be enabled at runtime. */
  qdrant_configured: boolean;
}

export interface PromptCreateInput {
  slug: string;
  name: string;
  system_prompt: string;
  user_template: string;
  model?: string | null;
  temperature?: number | null;
  max_tokens?: number | null;
  variables?: PromptVariable[];
  is_active?: boolean;
}

export interface UpdatePromptInput {
  name?: string;
  system_prompt?: string;
  user_template?: string;
  model?: string | null;
  temperature?: number | null;
  max_tokens?: number | null;
  variables?: PromptVariable[];
  is_active?: boolean;
}

export interface McpToolDto {
  name: string;
  description: string;
}

export interface McpServerDto {
  id: string;
  name: string;
  transport: string;
  url: string;
  headers: Record<string, string>;
  allowed_tools: string[];
  tools: McpToolDto[];
  tool_count: number;
  last_status: string | null;
  is_active: boolean;
  created_at: string;
  updated_at: string;
}

export interface McpToolEntryDto {
  server: string;
  name: string;
  description: string;
}

export interface CreateMcpServerInput {
  name: string;
  url: string;
  transport?: string;
  allowed_tools?: string[];
  is_active?: boolean;
}

export interface UpdateMcpServerInput {
  name?: string;
  url?: string;
  transport?: string;
  allowed_tools?: string[];
  is_active?: boolean;
}

export interface UserDto {
  id: string | null;
  email: string;
  name: string;
  role: 'owner' | 'admin' | 'operator' | 'viewer';
  is_active: boolean;
  created_at: string | null;
  last_login_at: string | null;
}

export interface CreateUserInput {
  email: string;
  name: string;
  password: string;
  role: 'owner' | 'admin' | 'operator' | 'viewer';
}

export interface UpdateUserInput {
  email?: string;
  name?: string;
  role?: 'owner' | 'admin' | 'operator' | 'viewer';
  is_active?: boolean;
}
