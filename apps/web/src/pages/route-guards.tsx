import { useEffect } from 'react';
import type { ReactNode } from 'react';

import type { Capability } from '@/api/auth';
import { can } from '@/api/auth';
import { useCurrentUser } from '@/api/current-user';
import { ForbiddenPage } from '@/pages/ForbiddenPage';
import { GuardrailsPage } from '@/pages/GuardrailsPage';
import { KeysPage } from '@/pages/KeysPage';
import { LogsPage } from '@/pages/LogsPage';
import { McpPage } from '@/pages/McpPage';
import { OverviewPage } from '@/pages/OverviewPage';
import { PlaygroundPage } from '@/pages/PlaygroundPage';
import { PromptsPage } from '@/pages/PromptsPage';
import { RoutingPage } from '@/pages/RoutingPage';
import { ProviderDetailPage } from '@/pages/ProviderDetailPage';
import { SettingsPage } from '@/pages/SettingsPage';
import { UsersPage } from '@/pages/UsersPage';
import { WebhooksPage } from '@/pages/WebhooksPage';

interface RequireCapabilityProps {
  capability: Capability;
  page: ReactNode;
}

export function RequireCapability({ capability, page }: RequireCapabilityProps) {
  const user = useCurrentUser();

  useEffect(() => {
    if (user === null) {
      const redirect = `${window.location.pathname}${window.location.search}`;
      window.location.assign(`/login?redirect=${encodeURIComponent(redirect)}`);
    }
  }, [user]);

  if (user === null) return null;
  if (!can(user, capability)) return <ForbiddenPage capability={capability} />;
  return <>{page}</>;
}

export function OverviewRoute() {
  return <RequireCapability capability="read_overview" page={<OverviewPage />} />;
}

export function KeysRoute() {
  return <RequireCapability capability="manage_keys" page={<KeysPage />} />;
}

export function RoutingRoute() {
  return <RequireCapability capability="manage_routing" page={<RoutingPage />} />;
}

export function ProviderDetailRoute() {
  return <RequireCapability capability="manage_routing" page={<ProviderDetailPage />} />;
}

export function GuardrailsRoute() {
  return <RequireCapability capability="manage_guardrails" page={<GuardrailsPage />} />;
}

export function WebhooksRoute() {
  return <RequireCapability capability="manage_webhooks" page={<WebhooksPage />} />;
}

export function LogsRoute() {
  return <RequireCapability capability="read_logs" page={<LogsPage />} />;
}

export function SettingsRoute() {
  return <RequireCapability capability="change_settings" page={<SettingsPage />} />;
}

export function PromptsRoute() {
  return <RequireCapability capability="manage_prompts" page={<PromptsPage />} />;
}

export function McpServersRoute() {
  return <RequireCapability capability="manage_mcp" page={<McpPage />} />;
}

export function PlaygroundRoute() {
  return <RequireCapability capability="read_config" page={<PlaygroundPage />} />;
}

export function UsersRoute() {
  return <RequireCapability capability="manage_users" page={<UsersPage />} />;
}
