import { useState } from "react";

import { Button } from "@astryxdesign/core/Button";
import { Icon } from "@astryxdesign/core/Icon";
import { Section } from "@astryxdesign/core/Section";
import { HStack } from "@astryxdesign/core/HStack";
import { Pagination } from "@astryxdesign/core/Pagination";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";
import { getCoreRowModel, useLegacyTable } from "@tanstack/react-table/legacy";
import { ArrowPathIcon, MagnifyingGlassIcon } from "@heroicons/react/24/outline";

import { api } from "@/api/client";
import type { RequestLogDto } from "@/api/types";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { usePageAction } from "@/components/common/use-page-action";
import { LogFiltersBar } from "@/components/logs/LogFiltersBar";
import type { LogFilters } from "@/components/logs/LogFiltersBar";
import { LogInspectorDrawer } from "@/components/logs/LogInspectorDrawer";
import { LogsTable } from "@/components/logs/LogsTable";
import { logColumns } from "@/components/logs/logColumns";
import { useQuery } from "@tanstack/react-query";
import { queryKeys } from "@/api/queryKeys";

export function LogsPage() {
  const [filters, setFilters] = useState<LogFilters>({
    search: "",
    status: "",
    model: "",
    pageSize: 50,
  });
  const [page, setPage] = useState(1);
  const [redactOverride, setRedactOverride] = useState<boolean | null>(null);
  const [selectedLog, setSelectedLog] = useState<RequestLogDto | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);

  const logsQuery = {
    page,
    page_size: filters.pageSize,
    status: filters.status || undefined,
    model: filters.model.trim() || undefined,
    search: filters.search.trim() || undefined,
  };

  const logsState = useQuery({
    queryKey: queryKeys.logs(JSON.stringify(logsQuery)),
    queryFn: () => api.listLogs(logsQuery),
  });

  const settingsState = useQuery({
    queryKey: queryKeys.settings,
    queryFn: api.getSettings,
  });
  const isRedacted =
    redactOverride ?? settingsState.data?.redact_prompts ?? false;
  const items = logsState.data?.items ?? [];
  const total = logsState.data?.total ?? 0;
  const hasFilters =
    filters.search.trim() !== "" || filters.status !== "" || filters.model.trim() !== "";

  const table = useLegacyTable({
    data: items,
    columns: logColumns,
    getCoreRowModel: getCoreRowModel(),
  });

  function applyFilters(next: LogFilters) {
    setFilters(next);
    setPage(1);
  }

  async function toggleRedaction(next: boolean) {
    setRedactOverride(next);
    setSettingsError(null);
    try {
      await api.putSettings({ redact_prompts: next });
    } catch (cause) {
      setSettingsError(cause instanceof Error ? cause.message : String(cause));
      setRedactOverride(!next);
    }
  }

  usePageAction(
    () => (
      <Button
        label="Refresh"
        variant="secondary"
        icon={<Icon icon={ArrowPathIcon} />}
        onClick={() => void logsState.refetch()}
      />
    ),
    [logsState.refetch],
  )

  return (
    <VStack gap={4}>
      <LogFiltersBar
        filters={filters}
        onChange={applyFilters}
        isRedacted={isRedacted}
        onRedactedChange={toggleRedaction}
      />

      <ErrorBanner message={logsState.error?.message ?? settingsError} />

      <Section padding={0}>
        {items.length === 0 && !logsState.isLoading ? (
          <EmptyState
            title="No requests match"
            description="Clear the search or filters to see every proxied request."
            headingLevel={3}
            icon={<Icon icon={MagnifyingGlassIcon} size="md" />}
            actions={
              hasFilters ? (
                <Button
                  label="Clear filters"
                  variant="secondary"
                  size="sm"
                  onClick={() =>
                    applyFilters({ search: "", status: "", model: "", pageSize: filters.pageSize })
                  }
                />
              ) : undefined
            }
          />
        ) : (
          <LogsTable table={table} onSelect={setSelectedLog} />
        )}
      </Section>

      {items.length > 0 ? (
        <HStack hAlign="between" vAlign="center" wrap="wrap" gap={3}>
          <Text size="sm" color="secondary" hasTabularNumbers>
            {`${(page - 1) * filters.pageSize + 1}–${(page - 1) * filters.pageSize + items.length} of ${total}`}
          </Text>
          <Pagination
            page={page}
            onChange={setPage}
            totalItems={total}
            pageSize={filters.pageSize}
            variant="compact"
            label="Log pages"
          />
        </HStack>
      ) : null}

      <LogInspectorDrawer
        log={selectedLog}
        isRedacted={isRedacted}
        onOpenChange={(isOpen) => {
          if (!isOpen) setSelectedLog(null);
        }}
      />
    </VStack>
  );
}
