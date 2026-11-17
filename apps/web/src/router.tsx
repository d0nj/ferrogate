import { createRootRoute, createRoute, createRouter } from '@tanstack/react-router'

import App from '@/App'
import { LoginPage } from '@/pages/LoginPage'
import {
  GuardrailsRoute,
  KeysRoute,
  LogsRoute,
  McpServersRoute,
  OverviewRoute,
  ProviderDetailRoute,
  PlaygroundRoute,
  PromptsRoute,
  RoutingRoute,
  SettingsRoute,
  UsersRoute,
  WebhooksRoute,
} from '@/pages/route-guards'

const rootRoute = createRootRoute({ component: App })

const routes = [
  createRoute({ getParentRoute: () => rootRoute, path: '/', component: OverviewRoute }),
  createRoute({ getParentRoute: () => rootRoute, path: '/login', component: LoginPage }),
  createRoute({ getParentRoute: () => rootRoute, path: '/keys', component: KeysRoute }),
  createRoute({ getParentRoute: () => rootRoute, path: '/routing', component: RoutingRoute }),
  createRoute({
    getParentRoute: () => rootRoute,
    path: '/routing/$providerId',
    component: ProviderDetailRoute,
  }),
  createRoute({ getParentRoute: () => rootRoute, path: '/guardrails', component: GuardrailsRoute }),
  createRoute({ getParentRoute: () => rootRoute, path: '/webhooks', component: WebhooksRoute }),
  createRoute({ getParentRoute: () => rootRoute, path: '/logs', component: LogsRoute }),
  createRoute({ getParentRoute: () => rootRoute, path: '/settings', component: SettingsRoute }),
  createRoute({ getParentRoute: () => rootRoute, path: '/prompts', component: PromptsRoute }),
  createRoute({ getParentRoute: () => rootRoute, path: '/mcp-servers', component: McpServersRoute }),
  createRoute({
    getParentRoute: () => rootRoute,
    path: '/playground',
    component: PlaygroundRoute,
  }),
  createRoute({ getParentRoute: () => rootRoute, path: '/users', component: UsersRoute }),
]

export const router = createRouter({
  routeTree: rootRoute.addChildren(routes),
  defaultPreload: 'intent',
})

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
