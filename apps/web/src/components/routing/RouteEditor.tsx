import { Badge } from '@astryxdesign/core/Badge';
import { Button } from '@astryxdesign/core/Button';
import { EmptyState } from '@astryxdesign/core/EmptyState';
import { HStack } from '@astryxdesign/core/Stack';
import { Icon } from '@astryxdesign/core/Icon';
import { IconButton } from '@astryxdesign/core/IconButton';
import { pixel, proportional, Table } from '@astryxdesign/core/Table';
import type { TableColumn } from '@astryxdesign/core/Table';
import { Text } from '@astryxdesign/core/Text';
import { PencilSquareIcon, TrashIcon } from '@heroicons/react/24/outline';

import type { Route, RouteTarget } from '@/api/types';

export interface RouteActions {
  onAdd: () => void;
  onEdit: (route: Route) => void;
  onDelete: (route: Route) => void;
}

interface RouteEditorProps extends RouteActions {
  routes: Route[];
}

interface RouteRow extends Record<string, unknown> {
  alias: string;
  route: Route;
}

function TargetChips({ targets }: { targets: RouteTarget[] }) {
  return (
    <HStack gap={1} wrap="wrap" hAlign="start">
      {targets.map((target, index) => (
        <Badge
          key={`${target.provider_id}:${target.upstream_model}:${index}`}
          variant={index === 0 ? 'blue' : 'neutral'}
          label={`${target.provider_name} → ${target.upstream_model}`}
        />
      ))}
    </HStack>
  );
}

function FallbackChips({ fallbacks }: { fallbacks: string[] }) {
  if (fallbacks.length === 0) {
    return (
      <Text size="sm" color="secondary">
        None
      </Text>
    );
  }

  return (
    <HStack gap={1} wrap="wrap" hAlign="start">
      {fallbacks.map((alias) => (
        <Badge key={alias} variant="purple" label={alias} />
      ))}
    </HStack>
  );
}

function RouteRowActions({ route, onEdit, onDelete }: { route: Route } & Omit<RouteActions, 'onAdd'>) {
  return (
    <HStack gap={1} justify="end">
      <IconButton
        label={`Edit ${route.alias}`}
        variant="ghost"
        size="sm"
        icon={<Icon icon={PencilSquareIcon} />}
        onClick={() => onEdit(route)}
      />
      <IconButton
        label={`Delete ${route.alias}`}
        variant="ghost"
        size="sm"
        icon={<Icon icon={TrashIcon} />}
        onClick={() => onDelete(route)}
      />
    </HStack>
  );
}

function columns({ onEdit, onDelete }: Omit<RouteActions, 'onAdd'>): TableColumn<RouteRow>[] {
  return [
    {
      key: 'alias',
      header: 'Alias',
      width: proportional(0.8, { minWidth: 120 }),
      renderCell: (row) => <Text weight="semibold">{row.route.alias}</Text>,
    },
    {
      key: 'strategy',
      header: 'Strategy',
      width: pixel(112),
      renderCell: (row) =>
        row.route.strategy ? (
          <Badge variant="blue" label={row.route.strategy} />
        ) : (
          <Text size="sm" color="secondary">
            Global policy
          </Text>
        ),
    },
    {
      key: 'targets',
      header: 'Attempt order',
      width: proportional(1.6, { minWidth: 220 }),
      renderCell: (row) => <TargetChips targets={row.route.targets} />,
    },
    {
      key: 'fallbacks',
      header: 'Fallbacks',
      width: proportional(0.8, { minWidth: 120 }),
      renderCell: (row) => <FallbackChips fallbacks={row.route.fallbacks} />,
    },
    {
      key: 'actions',
      header: '',
      width: pixel(84),
      renderCell: (row) => (
        <RouteRowActions route={row.route} onEdit={onEdit} onDelete={onDelete} />
      ),
    },
  ];
}

export function RouteEditor({ routes, onAdd, onEdit, onDelete }: RouteEditorProps) {
  if (routes.length === 0) {
    return (
      <EmptyState
        title="No routes yet."
        description="A route maps an alias to an ordered list of provider models."
        actions={<Button label="Add route" variant="secondary" onClick={onAdd} />}
      />
    );
  }

  const rows: RouteRow[] = routes.map((route) => ({ alias: route.alias, route }));

  return (
    <Table
      data={rows}
      idKey="alias"
      density="spacious"
      dividers="rows"
      hasHover
      columns={columns({ onEdit, onDelete })}
    />
  );
}
