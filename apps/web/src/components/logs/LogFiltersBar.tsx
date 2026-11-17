import { useMemo } from 'react';

import { HStack } from '@astryxdesign/core/Stack';
import { NumberInput } from '@astryxdesign/core/NumberInput';
import { Selector } from '@astryxdesign/core/Selector';
import { Switch } from '@astryxdesign/core/Switch';
import { TextInput } from '@astryxdesign/core/TextInput';
import { VStack } from '@astryxdesign/core/VStack';

const STATUS_OPTIONS = [
  { value: '', label: 'All statuses' },
  { value: '200', label: '200 OK' },
  { value: '400', label: '400 Bad request' },
  { value: '401', label: '401 Unauthorized' },
  { value: '403', label: '403 Forbidden' },
  { value: '429', label: '429 Rate limited' },
  { value: '502', label: '502 Bad gateway' },
  { value: '503', label: '503 Unavailable' },
];

export interface LogFilters {
  search: string;
  status: string;
  model: string;
  pageSize: number;
}

interface LogFiltersBarProps {
  filters: LogFilters;
  onChange: (filters: LogFilters) => void;
  isRedacted: boolean;
  onRedactedChange: (isRedacted: boolean) => void;
}

export function LogFiltersBar({
  filters,
  onChange,
  isRedacted,
  onRedactedChange,
}: LogFiltersBarProps) {
  const statusOptions = useMemo(() => STATUS_OPTIONS, []);

  return (
    <VStack gap={3}>
      <HStack gap={3} vAlign="end" wrap="wrap">
        <TextInput
          label="Search"
          value={filters.search}
          onChange={(search) => onChange({ ...filters, search })}
          placeholder="Request id, key or model"
          width={288}
        />
        <Selector
          label="Status"
          options={statusOptions}
          value={filters.status}
          onChange={(status) => onChange({ ...filters, status })}
          width={224}
        />
        <TextInput
          label="Model"
          value={filters.model}
          onChange={(model) => onChange({ ...filters, model })}
          placeholder="exact model"
          width={224}
        />
        <NumberInput
          label="Page size"
          value={filters.pageSize}
          min={10}
          max={200}
          step={10}
          isIntegerOnly
          onChange={(pageSize) => onChange({ ...filters, pageSize: pageSize ?? 50 })}
          width={128}
        />
      </HStack>
      <Switch label="Redact Prompts" value={isRedacted} onChange={onRedactedChange} />
    </VStack>
  );
}
