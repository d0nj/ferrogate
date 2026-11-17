import { Link as TanStackRouterLink } from '@tanstack/react-router'

import { Link } from '@astryxdesign/core/Link';

import { Badge } from '@astryxdesign/core/Badge';
import { EmptyState } from '@astryxdesign/core/EmptyState';
import { HStack } from '@astryxdesign/core/Stack';
import { Icon } from '@astryxdesign/core/Icon';
import { IconButton } from '@astryxdesign/core/IconButton';
import { pixel, proportional, Table } from '@astryxdesign/core/Table';
import type { TableColumn } from '@astryxdesign/core/Table';
import { Text } from '@astryxdesign/core/Text';
import { PencilSquareIcon, TrashIcon } from '@heroicons/react/24/outline';

import type { ProviderDto } from '@/api/types';

export interface ProviderActions {
  onEdit: (provider: ProviderDto) => void;
  onDelete: (provider: ProviderDto) => void;
}

interface ProvidersTableProps extends ProviderActions {
  providers: ProviderDto[];
}

interface ProviderRow extends Record<string, unknown> {
  id: string;
  provider: ProviderDto;
}

function ModelsCell({ models }: { models: string[] }) {
  return (
    <HStack gap={1} wrap="wrap" hAlign="start">
      {models.slice(0, 3).map((model) => (
        <Badge key={model} variant="neutral" label={model} />
      ))}
      {models.length > 3 ? <Badge variant="neutral" label={`+${models.length - 3}`} /> : null}
    </HStack>
  );
}

function ProviderRowActions({
  provider,
  onEdit,
  onDelete,
}: { provider: ProviderDto } & ProviderActions) {
  return (
    <HStack gap={1} justify="end">
      <IconButton
        label={`Edit ${provider.name}`}
        variant="ghost"
        size="sm"
        icon={<Icon icon={PencilSquareIcon} />}
        onClick={() => onEdit(provider)}
      />
      <IconButton
        label={`Delete ${provider.name}`}
        variant="ghost"
        size="sm"
        icon={<Icon icon={TrashIcon} />}
        onClick={() => onDelete(provider)}
      />
    </HStack>
  );
}

function columns({ onEdit, onDelete }: ProviderActions): TableColumn<ProviderRow>[] {
  return [
    {
      key: 'name',
      header: 'Provider',
      width: proportional(1.2, { minWidth: 140 }),
      renderCell: (row) => (
        <Link
          as={TanStackRouterLink}
          href={`/routing/${row.provider.id}`}
          color="inherit"
          size="sm"
          weight="semibold"
        >
          {row.provider.name}
        </Link>
      ),
    },
    {
      key: 'status',
      header: 'Status',
      width: pixel(104),
      renderCell: (row) => (
        <Badge
          variant={row.provider.is_active ? 'success' : 'neutral'}
          label={row.provider.is_active ? 'Active' : 'Paused'}
        />
      ),
    },
    {
      key: 'type',
      header: 'Type',
      width: proportional(0.6, { minWidth: 118 }),
      renderCell: (row) => (
        <HStack gap={1} wrap="wrap">
          <Badge variant="blue" label={row.provider.provider_type} />
          {row.provider.api_dialect === 'responses' ? (
            <Badge variant="purple" label={row.provider.api_dialect} />
          ) : null}
        </HStack>
      ),
    },
    {
      key: 'models',
      header: 'Models',
      width: proportional(1.4, { minWidth: 180 }),
      renderCell: (row) => <ModelsCell models={row.provider.models} />,
    },
    {
      key: 'apiKey',
      header: 'API key',
      width: proportional(0.6, { minWidth: 96 }),
      renderCell: (row) =>
        row.provider.has_api_key ? (
          <HStack gap={1} vAlign="center" wrap="wrap">
            <Text size="sm" color="secondary">
              {row.provider.api_key_masked}
            </Text>
            {row.provider.pool_key_count > 1 ? (
              <Badge
                variant="neutral"
                label={`+${row.provider.pool_key_count - 1}`}
              />
            ) : null}
          </HStack>
        ) : (
          <Badge variant="warning" label="Missing" />
        ),
    },
    {
      key: 'timeout',
      header: 'Timeout',
      width: pixel(88),
      align: 'end',
      renderCell: (row) => (
        <Text size="sm" hasTabularNumbers>
          {row.provider.timeout_ms} ms
        </Text>
      ),
    },
    {
      key: 'actions',
      header: '',
      width: pixel(84),
      renderCell: (row) => (
        <ProviderRowActions
          provider={row.provider}
          onEdit={onEdit}
          onDelete={onDelete}
        />
      ),
    },
  ];
}

export function ProvidersTable({ providers, onEdit, onDelete }: ProvidersTableProps) {
  if (providers.length === 0) {
    return <EmptyState title="No providers configured." />;
  }

  const rows: ProviderRow[] = providers.map((provider) => ({
    id: provider.id,
    provider,
  }));

  return (
    <Table
      data={rows}
      idKey="id"
      density="spacious"
      dividers="rows"
      hasHover
      columns={columns({ onEdit, onDelete })}
    />
  );
}
