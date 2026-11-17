/**
 * Every cache key the console reads through.
 *
 * Keys live in one place so a mutation invalidates the same thing a query
 * registered: a literal repeated across fifteen files drifts silently, and the
 * symptom is a stale table rather than an error.
 */
export const queryKeys = {
  providers: ['providers'] as const,
  providerKeys: (providerId: string) => ['providers', providerId, 'keys'] as const,
  routing: ['routing'] as const,
  routes: ['routes'] as const,
  keys: ['keys'] as const,
  teams: ['teams'] as const,
  guardrails: ['guardrails'] as const,
  webhooks: ['webhooks'] as const,
  users: ['users'] as const,
  prompts: ['prompts'] as const,
  prompt: (slug: string) => ['prompts', slug] as const,
  mcpServers: ['mcp', 'servers'] as const,
  mcpTools: ['mcp', 'tools'] as const,
  logs: (query: string) => ['logs', query] as const,
  log: (id: string) => ['logs', 'detail', id] as const,
  overview: (window: string) => ['overview', window] as const,
  cacheStats: ['settings', 'response-cache'] as const,
  semanticStats: ['settings', 'semantic'] as const,
  settings: ['settings'] as const,
  pricing: ['pricing'] as const,
  session: ['session'] as const,
}
