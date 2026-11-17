import { Badge } from '@astryxdesign/core/Badge';
import { HStack } from '@astryxdesign/core/HStack';
import { Text } from '@astryxdesign/core/Text';
import { VStack } from '@astryxdesign/core/VStack';
import type { LegacyColumnDef } from '@tanstack/react-table/legacy';

import type { RequestLogDto } from '@/api/types';
import { StatusBadge } from '@/components/logs/LogsTable';
import { formatMs, formatNumber, formatTime, formatUsd } from '@/utils/format';

export const logColumns: LegacyColumnDef<RequestLogDto>[] = [
  {
    id: 'created_at',
    header: 'Time',
    cell: ({ row }) => (
      <Text size="sm" color="secondary" hasTabularNumbers>
        {formatTime(row.original.created_at)}
      </Text>
    ),
  },
  {
    id: 'model',
    header: 'Requested model',
    cell: ({ row }) => (
      <VStack gap={0.5}>
        <Text size="sm" weight="semibold">
          {row.original.requested_model}
        </Text>
        <Text size="2xs" color="secondary">
          {row.original.routed_provider_name ?? 'unrouted'}
          {row.original.routed_model ? ` · ${row.original.routed_model}` : ''}
        </Text>
      </VStack>
    ),
  },
  {
    id: 'key',
    header: 'Key',
    cell: ({ row }) => (
      <Text size="sm" color="secondary">
        {row.original.virtual_key_name ?? '—'}
      </Text>
    ),
  },
  {
    id: 'status',
    header: 'Status',
    cell: ({ row }) => (
      <HStack gap={1} vAlign="center">
        <StatusBadge statusCode={row.original.status_code} />
        {row.original.fallback_count > 0 ? (
          <Badge variant="warning" label={`${row.original.fallback_count} fallback${row.original.fallback_count === 1 ? '' : 's'}`} />
        ) : null}
        {row.original.cache_hit ? <Badge variant="cyan" label="cache" /> : null}
      </HStack>
    ),
  },
  {
    id: 'tokens',
    header: 'Tokens',
    cell: ({ row }) => (
      <Text size="sm" hasTabularNumbers>
        {formatNumber(row.original.input_tokens)} / {formatNumber(row.original.output_tokens)}
      </Text>
    ),
  },
  {
    id: 'latency_ms',
    header: 'Latency',
    cell: ({ row }) => (
      <Text size="sm" hasTabularNumbers>
        {formatMs(row.original.latency_ms)}
      </Text>
    ),
  },
  {
    id: 'cost_usd',
    header: 'Cost',
    cell: ({ row }) => (
      <Text size="sm" hasTabularNumbers>
        {formatUsd(row.original.cost_usd)}
      </Text>
    ),
  },
];
