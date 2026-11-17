import { Fragment, useEffect, useState } from 'react'

import { AppShell } from '@astryxdesign/core/AppShell'
import { Divider } from '@astryxdesign/core/Divider'
import { Center } from '@astryxdesign/core/Center'
import { Layout, LayoutContent, VStack } from '@astryxdesign/core/Layout'
import { LayerProvider } from '@astryxdesign/core/Layer'
import {
  SideNav,
  SideNavHeading,
  SideNavItem,
  SideNavSection,
} from '@astryxdesign/core/SideNav'
import { Spinner } from '@astryxdesign/core/Spinner'
import { Theme } from '@astryxdesign/core/theme'
import {
  ArrowsRightLeftIcon,
  ArrowRightOnRectangleIcon,
  ComputerDesktopIcon,
  MoonIcon,
  SunIcon,
  BellAlertIcon,
  ChatBubbleLeftRightIcon,
  DocumentTextIcon,
  KeyIcon,
  RectangleStackIcon,
  ServerStackIcon,
  ShieldCheckIcon,
  AdjustmentsHorizontalIcon,
  Squares2X2Icon,
  UserCircleIcon,
  UsersIcon,
} from '@heroicons/react/24/outline'
import { Link, Outlet, useRouterState } from '@tanstack/react-router'
import type { ComponentProps, ComponentType, SVGProps } from 'react'

import type { Capability, CurrentUser } from '@/api/auth'
import { can, fetchSession, logout } from '@/api/auth'
import { CurrentUserProvider } from '@/api/current-user'
import { defaultThemeSlug, themes } from '@/astryx-themes'
import { ControlPlaneHeader } from '@/components/common/ControlPlaneHeader'
import { FerroGateLogo } from '@/components/common/FerroGateLogo'
import { PageActionProvider } from '@/components/common/page-actions'
import { useThemeMode } from '@/hooks/useThemeMode'
import type { ThemeMode } from '@/hooks/useThemeMode'

export interface NavEntry {
  to: string
  label: string
  capability: Capability
  icon: ComponentType<SVGProps<SVGSVGElement>>
}

interface NavGroup {
  title: string
  entries: NavEntry[]
}

/**
 * Grouped by what an operator is doing, not by feature: watch the gateway,
 * shape the traffic, then administer the console.
 */
const NAV_GROUPS: NavGroup[] = [
  {
    title: 'Monitor',
    entries: [
      { to: '/', label: 'Overview', capability: 'read_overview', icon: Squares2X2Icon },
      { to: '/logs', label: 'Request Logs', capability: 'read_logs', icon: RectangleStackIcon },
      { to: '/playground', label: 'Playground', capability: 'read_config', icon: ChatBubbleLeftRightIcon },
    ],
  },
  {
    title: 'Configure',
    entries: [
      { to: '/routing', label: 'Routing', capability: 'manage_routing', icon: ArrowsRightLeftIcon },
      { to: '/keys', label: 'Virtual Keys', capability: 'manage_keys', icon: KeyIcon },
      { to: '/guardrails', label: 'Guardrails', capability: 'manage_guardrails', icon: ShieldCheckIcon },
      { to: '/prompts', label: 'Prompts', capability: 'manage_prompts', icon: DocumentTextIcon },
      { to: '/mcp-servers', label: 'MCP', capability: 'manage_mcp', icon: ServerStackIcon },
      { to: '/webhooks', label: 'Webhooks', capability: 'manage_webhooks', icon: BellAlertIcon },
    ],
  },
  {
    title: 'Administer',
    entries: [
      { to: '/settings', label: 'Settings', capability: 'change_settings', icon: AdjustmentsHorizontalIcon },
      { to: '/users', label: 'Users', capability: 'manage_users', icon: UsersIcon },
    ],
  },
]

const NAV: NavEntry[] = NAV_GROUPS.flatMap((group) => group.entries)

function themeIcon(mode: ThemeMode) {
  if (mode === 'light') return SunIcon
  if (mode === 'system') return ComputerDesktopIcon
  return MoonIcon
}

/** Each mode spelled out in the account menu, instead of cycled through one icon. */
const THEME_OPTIONS = [
  { value: 'dark', label: 'Dark' },
  { value: 'light', label: 'Light' },
  { value: 'system', label: 'System' },
] as const satisfies readonly { value: ThemeMode; label: string }[]

type RouterLinkProps = Omit<ComponentProps<'a'>, 'href'> & { href?: string }

const RouterLink = ({ href = '/', ...rest }: RouterLinkProps) => (
  <Link to={href as never} {...(rest as Record<string, unknown>)} />
)

interface SessionState {
  status: 'idle' | 'loading' | 'ready'
  user: CurrentUser | null
}

export default function App() {
  const pathname = useRouterState({ select: (state) => state.location.pathname })
  const isLogin = pathname === '/login'
  const [session, setSession] = useState<SessionState>(() => ({
    status: 'loading',
    user: null,
  }))

  useEffect(() => {
    if (isLogin) return
    let isCurrent = true
    fetchSession()
      .then((user) => {
        if (!isCurrent) return
        if (!user) {
          window.location.assign('/login')
          return
        }
        setSession({ status: 'ready', user })
      })
      .catch(() => {
        if (isCurrent) setSession({ status: 'ready', user: null })
      })
    return () => {
      isCurrent = false
    }
  }, [isLogin])

  async function signOut() {
    await logout()
    window.location.assign('/login')
  }

  const { mode, setMode: setThemeMode } = useThemeMode()
  const page = NAV.find((entry) => entry.to === pathname)
  const user = session.status === 'ready' ? session.user : null

  const nav = (
    <SideNav
      aria-label="Control plane"
      collapsible
      header={
        <SideNavHeading
          heading="FerroGate"
          icon={<FerroGateLogo size={18} />}
        />
      }
      footer={
        user ? (
          <SideNavSection title="Account" isHeaderHidden>
            <SideNavItem label={user.name || user.email} icon={UserCircleIcon} />
            <SideNavItem
              label="Appearance"
              icon={themeIcon(mode)}
              collapsible={{ defaultIsCollapsed: true }}
            >
              {THEME_OPTIONS.map((option) => (
                <SideNavItem
                  key={option.value}
                  label={option.label}
                  isSelected={mode === option.value}
                  onClick={() => setThemeMode(option.value)}
                />
              ))}
            </SideNavItem>
            <SideNavItem
              label="Sign out"
              icon={ArrowRightOnRectangleIcon}
              onClick={() => void signOut()}
            />
          </SideNavSection>
        ) : undefined
      }
    >
      {NAV_GROUPS.map((group, index) => {
        const entries = group.entries.filter((entry) =>
          can(session.user, entry.capability),
        )
        // A group whose entries are all forbidden would leave a bare heading.
        if (entries.length === 0) return null
        return (
          <Fragment key={group.title}>
            {index > 0 && <Divider />}
            <SideNavSection title={group.title}>
              {entries.map((entry) => (
                <SideNavItem
                  key={entry.to}
                  label={entry.label}
                  icon={entry.icon}
                  size="sm"
                  href={entry.to}
                  as={RouterLink}
                  isSelected={pathname === entry.to}
                />
              ))}
            </SideNavSection>
          </Fragment>
        )
      })}
    </SideNav>
  )

  return (
    <Theme theme={themes[defaultThemeSlug]} mode={mode}>
      <LayerProvider>
        {isLogin ? (
          <Outlet />
        ) : session.status === 'loading' ? (
          <Center minHeight="100dvh">
            <Spinner label="Loading session" />
          </Center>
        ) : (
          <CurrentUserProvider user={session.user}>
            <PageActionProvider>
              <AppShell
                sideNav={nav}
                mobileNav={{ hasToggle: true, breakpoint: 'md' }}
              >
                {/* padding={0}: Table bleeds past the container inset; the VStack owns padding. */}
                <Layout
                  height="fill"
                  content={
                    <LayoutContent padding={0}>
                      <VStack padding={4} gap={6}>
                        <ControlPlaneHeader page={page} />
                        <Outlet />
                      </VStack>
                    </LayoutContent>
                  }
                />
              </AppShell>
            </PageActionProvider>
          </CurrentUserProvider>
        )}
      </LayerProvider>
    </Theme>
  )
}
