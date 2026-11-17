import { useState } from "react";

import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { HStack } from "@astryxdesign/core/HStack";
import { Icon } from "@astryxdesign/core/Icon";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Heading } from "@astryxdesign/core/Text";
import { Section } from "@astryxdesign/core/Section";
import { VStack } from "@astryxdesign/core/VStack";
import { ArrowPathIcon, TrashIcon } from "@heroicons/react/24/outline";

import { api } from "@/api/client";
import { queryKeys } from "@/api/queryKeys";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { formatNumber } from "@/utils/format";
import { useQuery, useQueryClient } from "@tanstack/react-query";

export function CachePanel() {
  const queryClient = useQueryClient();
  const cacheState = useQuery({
    queryKey: queryKeys.cacheStats,
    queryFn: api.getCacheStats,
  });
  const { data, isLoading } = cacheState;
  const error = cacheState.error?.message ?? null;
  const [actionError, setActionError] = useState<string | null>(null);
  const [isClearing, setIsClearing] = useState(false);
  const [clearedAt, setClearedAt] = useState<string | null>(null);

  async function clearCache() {
    setActionError(null);
    setClearedAt(null);
    setIsClearing(true);
    try {
      await api.clearCache();
      setClearedAt(new Date().toLocaleTimeString());
      void queryClient.invalidateQueries({ queryKey: queryKeys.cacheStats });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsClearing(false);
    }
  }

  const stats = data;

  return (
    <Section padding={4}>
      <VStack gap={3}>
        <HStack hAlign="between" vAlign="center" gap={4} wrap="wrap">
          <VStack gap={0.5}>
            <Heading level={3}>Response cache</Heading>
            <Text type="supporting" size="sm">
              Deterministic completions served from memory
              {stats?.redis_connected
                ? " with Redis persistence"
                : stats?.redis_configured
                  ? " (Redis configured, not connected)"
                  : " (Redis disabled: REDIS_URL is not set)"}.
              {clearedAt ? ` Cleared at ${clearedAt}.` : ""}
            </Text>
          </VStack>
          <HStack gap={2} vAlign="center">
            <Button
              label="Refresh"
              variant="ghost"
              size="sm"
              icon={<Icon icon={ArrowPathIcon} />}
              onClick={() => void cacheState.refetch()}
              isLoading={isLoading}
            />
            <Button
              label="Clear cache"
              variant="secondary"
              size="sm"
              icon={<Icon icon={TrashIcon} />}
              onClick={clearCache}
              isLoading={isClearing}
            />
          </HStack>
        </HStack>
        <ErrorBanner message={error ?? actionError} />
        <Table
          data={[
            {
              metric: "Cached entries",
              value: stats ? formatNumber(stats.entries) : "—",
            },
            {
              metric: "Cache hits",
              value: stats ? formatNumber(stats.hits) : "—",
            },
            {
              metric: "Cache misses",
              value: stats ? formatNumber(stats.misses) : "—",
            },
            {
              metric: "Redis",
              value: !stats
                ? "—"
                : !stats.redis_configured
                  ? "disabled"
                  : stats.redis_connected
                    ? "connected"
                    : "unreachable",
            },
          ]}
          density="spacious"
          dividers="rows"
          columns={[
            {
              key: "metric",
              header: "Metric",
              width: proportional(1, { minWidth: 140 }),
              renderCell: (row) => (
                <Text weight="semibold">{String(row.metric)}</Text>
              ),
            },
            {
              key: "value",
              header: "Value",
              align: "end",
              width: pixel(160),
              renderCell: (row) =>
                row.metric === "Redis" ? (
                  <Badge
                    variant={
                      row.value === "connected"
                        ? "success"
                        : row.value === "unreachable"
                          ? "warning"
                          : "neutral"
                    }
                    label={String(row.value)}
                  />
                ) : (
                  <Text size="sm" hasTabularNumbers>
                    {String(row.value)}
                  </Text>
                ),
            },
          ]}
          emptyState={null}
        />
      </VStack>
    </Section>
  );
}
