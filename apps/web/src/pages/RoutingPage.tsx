import { useState } from "react";

import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { Icon } from "@astryxdesign/core/Icon";
import { Section } from "@astryxdesign/core/Section";
import { Tab, TabList } from "@astryxdesign/core/TabList";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";
import { PlusIcon } from "@heroicons/react/24/outline";

import { api } from "@/api/client";
import { RoutingPolicyPanel } from "@/components/routing/RoutingPolicyPanel";
import type { ProviderDto, Route } from "@/api/types";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { useToast } from "@astryxdesign/core/Toast";
import { usePageAction } from "@/components/common/use-page-action";
import { ProviderDialog } from "@/components/routing/ProviderDialog";
import { ProvidersTable } from "@/components/routing/ProvidersPanel";
import { RouteDialog } from "@/components/routing/RouteDialog";
import { RouteEditor } from "@/components/routing/RouteEditor";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { queryKeys } from "@/api/queryKeys";

type RoutingTab = "providers" | "routes" | "policy";

export function RoutingPage() {
  const showToast = useToast();
  const queryClient = useQueryClient();
  const { ask, dialog } = useConfirmDialog();
  const [tab, setTab] = useState<RoutingTab>("providers");
  const providersState = useQuery({
    queryKey: queryKeys.providers,
    queryFn: api.listProviders,
  });
  const routesState = useQuery({
    queryKey: queryKeys.routes,
    queryFn: api.listRoutes,
  });
  const policyState = useQuery({
    queryKey: [...queryKeys.routing, "policy"],
    queryFn: api.getRoutingPolicy,
  });
  const [actionError, setActionError] = useState<string | null>(null);

  const [providerDialog, setProviderDialog] = useState<{
    isOpen: boolean;
    provider: ProviderDto | null;
  }>({ isOpen: false, provider: null });
  const [routeDialog, setRouteDialog] = useState<{
    isOpen: boolean;
    route: Route | null;
  }>({
    isOpen: false,
    route: null,
  });

  const providers = providersState.data ?? [];
  const routes = routesState.data ?? [];
  const aliases = routes.map((route) => route.alias);

  function reloadAll() {
    void queryClient.invalidateQueries({ queryKey: queryKeys.providers });
    void queryClient.invalidateQueries({ queryKey: queryKeys.routes });
  }

  async function deleteProviderNow(provider: ProviderDto) {
    setActionError(null);
    try {
      await api.deleteProvider(provider.id);
      reloadAll();
      showToast({ body: "Provider deleted", uniqueID: `delete-provider-${provider.id}` });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  function deleteProvider(provider: ProviderDto) {
    ask({
      title: `Delete ${provider.name}?`,
      description:
        "Its route targets and provider keys are removed with it. This cannot be undone.",
      actionLabel: "Delete provider",
      onConfirm: () => deleteProviderNow(provider),
    });
  }

  async function deleteRouteNow(route: Route) {
    setActionError(null);
    try {
      await api.deleteRoute(route.alias);
      reloadAll();
      showToast({ body: "Route deleted", uniqueID: `delete-route-${route.alias}` });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  function deleteRoute(route: Route) {
    ask({
      title: `Delete route ${route.alias}?`,
      description: "Requests for this alias stop resolving. This cannot be undone.",
      actionLabel: "Delete route",
      onConfirm: () => deleteRouteNow(route),
    });
  }

  usePageAction(
    () =>
      // The policy tab edits in place, so it has no create action.
      tab === "policy" ? null : (
        <Button
          label={tab === "providers" ? "Add provider" : "Add route"}
          variant="primary"
          icon={<Icon icon={PlusIcon} />}
          onClick={() =>
            tab === "providers"
              ? setProviderDialog({ isOpen: true, provider: null })
              : setRouteDialog({ isOpen: true, route: null })
          }
        />
      ),
    [tab],
  )

  return (
    <VStack gap={4}>
      {dialog}
      <TabList
        value={tab}
        onChange={(value) => setTab(value as RoutingTab)}
        hasDivider
      >
        <Tab value="providers" label="Providers" />
        <Tab
          value="routes"
          label="Alias Routing"
          endContent={<Badge variant="neutral" label={`${routes.length}`} />}
        />
        <Tab value="policy" label="Policy" />
      </TabList>

      <ErrorBanner
        message={
          providersState.error?.message ??
          routesState.error?.message ??
          actionError
        }
      />

      {tab === "policy" ? (
        <RoutingPolicyPanel />
      ) : tab === "providers" ? (
        <VStack gap={4}>
          <Section padding={0}>
            <ProvidersTable
              providers={providers}
              onEdit={(provider) =>
                setProviderDialog({ isOpen: true, provider })
              }
              onDelete={deleteProvider}
            />
          </Section>
        </VStack>
      ) : (
        <VStack gap={4}>
          <Text type="supporting" size="sm">
            Each alias lists its targets in attempt order; fallbacks are tried
            when every target fails.
          </Text>
          <Section padding={0}>
            <RouteEditor
              routes={routes}
              onAdd={() => setRouteDialog({ isOpen: true, route: null })}
              onEdit={(route) => setRouteDialog({ isOpen: true, route })}
              onDelete={deleteRoute}
            />
          </Section>
        </VStack>
      )}

      {providerDialog.isOpen ? (
        <ProviderDialog
          key={providerDialog.provider?.id ?? "new-provider"}
          isOpen
          provider={providerDialog.provider}
          onOpenChange={(isOpen) => {
            if (!isOpen) setProviderDialog({ isOpen: false, provider: null });
          }}
          onSaved={() => {
            setProviderDialog({ isOpen: false, provider: null });
            reloadAll();
          }}
        />
      ) : null}

      {routeDialog.isOpen ? (
        <RouteDialog
          key={routeDialog.route?.alias ?? "new-route"}
          isOpen
          route={routeDialog.route}
          providers={providers}
          aliases={aliases}
          globalStrategy={policyState.data?.strategy ?? null}
          onOpenChange={(isOpen) => {
            if (!isOpen) setRouteDialog({ isOpen: false, route: null });
          }}
          onSaved={() => {
            setRouteDialog({ isOpen: false, route: null });
            reloadAll();
          }}
        />
      ) : null}
    </VStack>
  );
}
