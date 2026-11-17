import { useMemo } from "react";

import { Section } from "@astryxdesign/core/Section";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Table, pixel, proportional } from "@astryxdesign/core/Table";
import type { TableColumn, TablePlugin } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";

import type { ProviderHealth } from "@/api/types";
import { formatNumber } from "@/utils/format";

interface HealthRow extends Record<string, unknown> {
  id: string;
  name: string;
  provider_type: string;
  circuit_state: string;
  is_active: boolean;
  consecutive_failures: number;
  total_requests: number;
  total_failures: number;
  last_error: string | null;
}

const CIRCUIT_STATUS: Record<
  string,
  { variant: "success" | "warning" | "error"; label: string }
> = {
  closed: { variant: "success", label: "Closed" },
  half_open: { variant: "warning", label: "Half-open" },
  open: { variant: "error", label: "Open" },
};

function toRow(provider: ProviderHealth): HealthRow {
  return {
    id: provider.provider_id,
    name: provider.name,
    provider_type: provider.provider_type,
    circuit_state: provider.circuit_state,
    is_active: provider.is_active,
    consecutive_failures: provider.consecutive_failures,
    total_requests: provider.total_requests,
    total_failures: provider.total_failures,
    last_error: provider.last_error,
  };
}

const COLUMNS: TableColumn<HealthRow>[] = [
  {
    key: "name",
    header: "Provider",
    width: proportional(1, { minWidth: 160 }),
    renderCell: (row) => (
      <VStack gap={0.5}>
        <Text weight="semibold">{row.name}</Text>
        <Text type="supporting">{row.provider_type}</Text>
      </VStack>
    ),
  },
  {
    key: "circuit_state",
    header: "Circuit",
    width: pixel(150),
    renderCell: (row) => {
      const status = CIRCUIT_STATUS[row.circuit_state] ?? {
        variant: "warning" as const,
        label: row.circuit_state,
      };
      return (
        <HStack gap={2} vAlign="center">
          <StatusDot variant={status.variant} label={status.label} />
          <Text
            type="supporting"
            color={status.variant === "error" ? "accent" : undefined}
            weight={status.variant === "error" ? "semibold" : undefined}
          >
            {status.label}
          </Text>
        </HStack>
      );
    },
  },
  {
    key: "is_active",
    header: "State",
    width: pixel(110),
    renderCell: (row) =>
      row.is_active ? (
        <Text type="supporting">Active</Text>
      ) : (
        <HStack gap={2} vAlign="center">
          <StatusDot variant="neutral" label="Inactive" />
          <Text type="supporting">Inactive</Text>
        </HStack>
      ),
  },
  {
    key: "total_requests",
    header: "Requests",
    align: "end",
    width: pixel(110),
    renderCell: (row) => (
      <Text hasTabularNumbers>{formatNumber(row.total_requests)}</Text>
    ),
  },
  {
    key: "total_failures",
    header: "Failures",
    align: "end",
    width: pixel(110),
    renderCell: (row) => (
      <Text
        hasTabularNumbers
        weight={row.total_failures > 0 ? "medium" : undefined}
        color={row.total_failures > 0 ? "primary" : "secondary"}
      >
        {formatNumber(row.total_failures)}
        {row.consecutive_failures > 0 ? ` (${row.consecutive_failures})` : ""}
      </Text>
    ),
  },
  {
    key: "last_error",
    header: "Last error",
    width: proportional(1.5, { minWidth: 200 }),
    renderCell: (row) =>
      row.last_error ? (
        <Text type="supporting" maxLines={1}>
          {row.last_error}
        </Text>
      ) : (
        <Text type="supporting" color="disabled">
          —
        </Text>
      ),
  },
];

export function ProviderHealthTable({
  providers,
}: {
  providers: ProviderHealth[];
}) {
  const attentionRows = useMemo<TablePlugin<HealthRow>>(
    () => ({
      rowModel: "flat",
      transformBodyRow: (props, item) =>
        item.circuit_state === "open"
          ? {
              ...props,
              htmlProps: { ...props.htmlProps, "data-row-attention": "true" },
            }
          : props,
    }),
    [],
  );

  return (
    <Section padding={0} dividers={["bottom"]}>
      <Table
        data={providers.map(toRow)}
        columns={COLUMNS}
        idKey="id"
        density="spacious"
        dividers="rows"
        hasHover
        textOverflow="truncate"
        plugins={{ attentionRows }}
      />
    </Section>
  );
}
