import { Badge } from '@astryxdesign/core/Badge';
import { Code } from '@astryxdesign/core/Code';
import { EmptyState } from '@astryxdesign/core/EmptyState';
import { HStack } from '@astryxdesign/core/HStack';
import { Icon } from '@astryxdesign/core/Icon';
import { IconButton } from '@astryxdesign/core/IconButton';
import { ProgressBar } from '@astryxdesign/core/ProgressBar';
import { Switch } from '@astryxdesign/core/Switch';
import { pixel, proportional, Table } from '@astryxdesign/core/Table';
import type { TableColumn } from '@astryxdesign/core/Table';
import { Text } from '@astryxdesign/core/Text';
import { VStack } from '@astryxdesign/core/VStack';
import { PencilSquareIcon, TrashIcon } from '@heroicons/react/24/outline';

import type { VirtualKeyDto } from '@/api/types';
import { formatNumber, formatTime, formatUsd } from '@/utils/format';

type KeyRow = VirtualKeyDto & Record<string, unknown>;

interface KeysTableProps {
  keys: VirtualKeyDto[];
  onToggleActive: (key: VirtualKeyDto, isActive: boolean) => void;
  onEdit: (key: VirtualKeyDto) => void;
  onRevoke: (key: VirtualKeyDto) => void;
}

const columns: TableColumn<KeyRow>[] = [
  {
    key: 'name',
    header: 'Name',
    width: proportional(1.2, { minWidth: 160 }),
    renderCell: (item) => (
      <VStack gap={0.5}>
        <Text weight="semibold">{item.name}</Text>
        <Text type="supporting" size="2xs">
          {item.last_used_at ? `Used ${formatTime(item.last_used_at)}` : 'Never used'}
        </Text>
      </VStack>
    ),
  },
  {
    key: 'budget_usd',
    header: 'Spend · Budget',
    width: proportional(1.1, { minWidth: 190 }),
    renderCell: (item) => <SpendCell item={item} />,
  },
  {
    key: 'key_prefix',
    header: 'Key',
    width: proportional(1, { minWidth: 150 }),
    renderCell: (item) => <Code color="secondary">{item.key_prefix}</Code>,
  },
  {
    key: 'rpm_limit',
    header: 'RPM · TPM',
    width: proportional(0.9, { minWidth: 120 }),
    renderCell: (item) => <LimitsCell item={item} />,
  },
  {
    key: 'allowed_models',
    header: 'Allowed models',
    width: proportional(1, { minWidth: 150 }),
    renderCell: (item) => <ModelsCell item={item} />,
  },
];

function SpendCell({ item }: { item: VirtualKeyDto }) {
  const budget = item.budget_usd;

  if (!budget) {
    return (
      <HStack gap={2} vAlign="center">
        <Text size="sm" hasTabularNumbers>
          {formatUsd(item.spent_usd)}
        </Text>
        <Badge variant="neutral" label="Unmetered" />
      </HStack>
    );
  }

  const ratio = item.spent_usd / budget;
  return (
    <VStack gap={1}>
      <Text size="sm" hasTabularNumbers>
        {formatUsd(item.spent_usd)} of {formatUsd(budget)}
      </Text>
      <ProgressBar
        label={`${item.name} spend against budget`}
        isLabelHidden
        value={Math.min(ratio, 1) * 100}
        variant={ratio >= 1 ? 'error' : ratio >= 0.8 ? 'warning' : 'accent'}
      />
    </VStack>
  );
}

function LimitsCell({ item }: { item: VirtualKeyDto }) {
  return (
    <VStack gap={0.5}>
      <Text size="sm" hasTabularNumbers>
        {item.rpm_limit ? `${formatNumber(item.rpm_limit)} RPM` : 'RPM ∞'}
      </Text>
      <Text size="sm" color="secondary" hasTabularNumbers>
        {item.tpm_limit ? `${formatNumber(item.tpm_limit)} TPM` : 'TPM ∞'}
      </Text>
    </VStack>
  );
}

function ModelsCell({ item }: { item: VirtualKeyDto }) {
  if (item.allowed_models.length === 0) {
    return (
      <Text size="sm" color="secondary">
        All models
      </Text>
    );
  }

  return (
    <HStack gap={1} wrap="wrap">
      {item.allowed_models.slice(0, 2).map((model) => (
        <Badge key={model} variant="blue" label={model} />
      ))}
      {item.allowed_models.length > 2 ? (
        <Badge variant="neutral" label={`+${item.allowed_models.length - 2}`} />
      ) : null}
    </HStack>
  );
}

function KeyActions({
  item,
  onEdit,
  onRevoke,
}: {
  item: VirtualKeyDto;
  onEdit: (key: VirtualKeyDto) => void;
  onRevoke: (key: VirtualKeyDto) => void;
}) {
  return (
    <HStack gap={1} vAlign="center" justify="end">
      <IconButton
        label="Edit key"
        tooltip="Edit key"
        variant="ghost"
        size="sm"
        icon={<Icon icon={PencilSquareIcon} />}
        onClick={() => onEdit(item)}
      />
      <IconButton
        label="Revoke key"
        tooltip="Revoke key"
        variant="ghost"
        size="sm"
        icon={<Icon icon={TrashIcon} />}
        onClick={() => onRevoke(item)}
      />
    </HStack>
  );
}

export function KeysTable({ keys, onToggleActive, onEdit, onRevoke }: KeysTableProps) {
  return (
    <Table
      data={keys as KeyRow[]}
      idKey="id"
      density="spacious"
      dividers="rows"
      hasHover
      textOverflow="truncate"
      columns={[
        ...columns,
        {
          key: 'active',
          header: 'Active',
          width: pixel(80),
          renderCell: (item) => (
            <Switch
              label={`${item.name} active`}
              isLabelHidden
              size="sm"
              value={item.is_active}
              onChange={(isActive) => onToggleActive(item, isActive)}
            />
          ),
        },
        {
          key: 'actions',
          header: '',
          width: pixel(96),
          align: 'end',
          renderCell: (item) => <KeyActions item={item} onEdit={onEdit} onRevoke={onRevoke} />,
        },
      ]}
      emptyState={
        <EmptyState
          title="No virtual keys yet"
          description="Create a virtual key to start issuing credentials."
          isCompact
        />
      }
    />
  );
}
