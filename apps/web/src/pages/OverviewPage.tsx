import { useState } from "react";

import { Button } from "@astryxdesign/core/Button";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { Icon } from "@astryxdesign/core/Icon";
import { ArrowPathIcon } from "@heroicons/react/24/outline";

import { api } from "@/api/client";
import type { OverviewWindow } from "@/api/types";
import { queryKeys } from "@/api/queryKeys";
import { KpiGrid } from "@/components/overview/KpiGrid";
import { ProviderHealthTable } from "@/components/overview/ProviderHealthTable";
import { UsageCharts } from "@/components/overview/UsageCharts";
import { WindowPicker } from "@/components/overview/WindowPicker";
import { useQuery } from "@tanstack/react-query";

export function OverviewPage() {
  const [window, setWindow] = useState<OverviewWindow>("24h");
  const overviewState = useQuery({
    queryKey: queryKeys.overview(window),
    queryFn: () => api.getOverview(window),
  });
  const { data, isLoading } = overviewState;

  return (
    <VStack gap={4}>
      <HStack gap={2} vAlign="center" justify="end" wrap="wrap">
        <WindowPicker value={window} onChange={setWindow} />
        <Button
          label="Refresh"
          variant="secondary"
          size="sm"
          icon={<Icon icon={ArrowPathIcon} />}
          isLoading={isLoading}
          onClick={() => void overviewState.refetch()}
        />
      </HStack>
      <ErrorBanner message={overviewState.error?.message ?? null} />
      <KpiGrid kpis={data?.kpis ?? null} />
      <UsageCharts buckets={data?.timeseries ?? []} isLoading={isLoading} />
      <ProviderHealthTable providers={data?.providers_health ?? []} />
    </VStack>
  );
}
