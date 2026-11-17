export type Role = 'owner' | 'admin' | 'operator' | 'viewer';

export interface CurrentUser {
  id: string | null;
  email: string;
  name: string;
  role: Role;
  is_active: boolean;
  created_at: string | null;
  last_login_at: string | null;
}

export type Capability =
  | 'read_overview'
  | 'read_logs'
  | 'read_config'
  | 'manage_providers'
  | 'manage_routing'
  | 'manage_keys'
  | 'manage_guardrails'
  | 'manage_webhooks'
  | 'manage_prompts'
  | 'manage_mcp'
  | 'manage_pricing'
  | 'manage_users'
  | 'clear_cache'
  | 'clear_logs'
  | 'change_settings';

const VIEWER_CAPABILITIES: readonly Capability[] = ['read_overview', 'read_logs', 'read_config'];

const OPERATOR_CAPABILITIES: readonly Capability[] = [
  ...VIEWER_CAPABILITIES,
  'manage_providers',
  'manage_routing',
  'manage_keys',
  'manage_guardrails',
  'manage_webhooks',
  'manage_prompts',
  'manage_mcp',
  'clear_cache',
  'clear_logs',
];

const ADMIN_CAPABILITIES: readonly Capability[] = [
  ...OPERATOR_CAPABILITIES,
  'manage_pricing',
  'change_settings',
];

const OWNER_CAPABILITIES: readonly Capability[] = [...ADMIN_CAPABILITIES, 'manage_users'];

const CAPABILITIES_BY_ROLE: Record<Role, readonly Capability[]> = {
  viewer: VIEWER_CAPABILITIES,
  operator: OPERATOR_CAPABILITIES,
  admin: ADMIN_CAPABILITIES,
  owner: OWNER_CAPABILITIES,
};

export function roleRank(role: Role): number {
  switch (role) {
    case 'owner':
      return 3;
    case 'admin':
      return 2;
    case 'operator':
      return 1;
    case 'viewer':
      return 0;
  }
}

export function can(user: CurrentUser | null, capability: Capability): boolean {
  if (!user) return false;
  return CAPABILITIES_BY_ROLE[user.role].includes(capability);
}

function bounceToLogin(): void {
  if (typeof window === 'undefined') return;
  if (window.location.pathname === '/login') return;
  const redirect = `${window.location.pathname}${window.location.search}`;
  window.location.assign(`/login?redirect=${encodeURIComponent(redirect)}`);
}

export async function fetchSession(): Promise<CurrentUser | null> {
  const res = await apiFetch('/admin/api/session');
  if (!res.ok) return null;
  return (await res.json()) as CurrentUser;
}

export async function apiFetch(path: string, init?: RequestInit): Promise<Response> {
  const res = await fetch(path, init);
  if (res.status === 401 && path.startsWith('/admin/api/') && !path.startsWith('/admin/api/session')) {
    bounceToLogin();
  }
  return res;
}

async function readErrorMessage(res: Response, fallback: string): Promise<string> {
  try {
    const body = (await res.json()) as { error?: { message?: string } };
    if (body?.error?.message) {
      return body.error.message;
    }
  } catch {
    // ignore non-JSON error body
  }
  return fallback;
}

export interface LoginResponse {
  expires_in: number;
}

async function completeLogin(res: Response): Promise<LoginResponse> {
  if (!res.ok) {
    throw new Error(await readErrorMessage(res, `Login failed (${res.status})`));
  }
  return (await res.json()) as LoginResponse;
}

export async function loginWithCredentials(email: string, password: string): Promise<LoginResponse> {
  const res = await fetch('/admin/api/session/login', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email, password }),
  });
  return completeLogin(res);
}

export async function logout(): Promise<void> {
  try {
    await apiFetch('/admin/api/session', { method: 'DELETE' });
  } catch {
    // Logout is best-effort; the cookie expires server-side regardless.
  }
  bounceToLogin();
}
