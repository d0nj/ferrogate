import { useEffect, useState } from 'react';

import { Badge } from '@astryxdesign/core/Badge';
import { Banner } from '@astryxdesign/core/Banner';
import { Button } from '@astryxdesign/core/Button';
import { Section } from '@astryxdesign/core/Section';
import { HStack } from '@astryxdesign/core/HStack';
import { Icon } from '@astryxdesign/core/Icon';
import { IconButton } from '@astryxdesign/core/IconButton';
import { pixel, proportional, Table } from '@astryxdesign/core/Table';
import type { TableColumn } from '@astryxdesign/core/Table';
import { Text } from '@astryxdesign/core/Text';
import { VStack } from '@astryxdesign/core/VStack';
import { PencilSquareIcon, PlusIcon, TrashIcon } from '@heroicons/react/24/outline';

import { api } from '@/api/client';
import { queryKeys } from '@/api/queryKeys';
import type { ProviderDto, ProviderKeyDto, ProviderTestResult } from '@/api/types';
import { ErrorBanner } from '@/components/common/ErrorBanner';
import { useConfirmDialog } from '@/hooks/useConfirmDialog';
import { useToast } from '@astryxdesign/core/Toast';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { formatTime } from '@/utils/format';
import { ProviderKeyDialog } from './ProviderKeyDialog';

type KeyRow = ProviderKeyDto & Record<string, unknown>;

interface ProviderKeysSectionProps {
  provider: ProviderDto;
}

function CooldownCell({ item }: { item: ProviderKeyDto }) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!item.in_cooldown || !item.cooldown_until) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [item.in_cooldown, item.cooldown_until]);

  if (!item.in_cooldown) {
    return <Text size="sm" color="secondary">Ready</Text>;
  }
  if (!item.cooldown_until) {
    return <Badge variant="warning" label="Cooling down" />;
  }
  const left = Math.max(0, new Date(item.cooldown_until).getTime() - now);
  if (left <= 0) {
    return <Badge variant="warning" label="Cooling down" />;
  }
  return <Badge variant="warning" label={`Cooldown ${Math.ceil(left / 1000)}s`} />;
}

const keyColumns: TableColumn<KeyRow>[] = [
  {
    key: 'name',
    header: 'Credential',
    width: proportional(1.1, { minWidth: 150 }),
    renderCell: (key) => (
      <VStack gap={0.5}>
        <Text weight="semibold">{key.name}</Text>
        <Text type="supporting" size="2xs">
          {key.has_api_key ? key.api_key_masked : 'No key stored'}
        </Text>
      </VStack>
    ),
  },
  {
    key: 'weight',
    header: 'Weight',
    align: 'end',
    width: pixel(80),
    renderCell: (key) => (
      <Text size="sm" hasTabularNumbers>
        {key.weight}
      </Text>
    ),
  },
  {
    key: 'models',
    header: 'Models',
    width: proportional(1.2, { minWidth: 140 }),
    renderCell: (key) =>
      key.models.length > 0 ? (
        <HStack gap={1} wrap="wrap">
          {key.models.slice(0, 3).map((model) => (
            <Badge key={model} variant="neutral" label={model} />
          ))}
          {key.models.length > 3 ? (
            <Badge variant="neutral" label={`+${key.models.length - 3}`} />
          ) : null}
        </HStack>
      ) : (
        <Text type="supporting" size="sm">
          Provider catalogue
        </Text>
      ),
  },
  {
    key: 'rpm_limit',
    header: 'RPM',
    align: 'end',
    width: pixel(90),
    renderCell: (key) => (
      <Text size="sm" hasTabularNumbers>
        {key.rpm_limit ?? '—'}
      </Text>
    ),
  },
  {
    key: 'in_cooldown',
    header: 'Status',
    width: pixel(130),
    renderCell: (key) => <CooldownCell item={key} />,
  },
  {
    key: 'last_error',
    header: 'Last error',
    width: proportional(1, { minWidth: 140 }),
    renderCell: (key) =>
      key.last_error ? (
        <Text size="sm" color="secondary">
          {key.last_error}
        </Text>
      ) : (
        <Text type="supporting" size="sm">
          —
        </Text>
      ),
  },
  {
    key: 'is_active',
    header: 'Active',
    width: pixel(80),
    renderCell: (key) => (
      <Badge variant={key.is_active ? 'success' : 'neutral'} label={key.is_active ? 'On' : 'Off'} />
    ),
  },
];

function formatTestResult(result: ProviderTestResult): string {
  if (!result.ok) return result.error ?? `Upstream status ${result.status_code ?? 'unknown'}`;
  const models =
    result.models_found === 1 ? '1 model' : `${result.models_found} models`;
  return `OK · HTTP ${result.status_code ?? '—'} · ${result.latency_ms} ms · ${models}`;
}

export function ProviderKeysSection({ provider }: ProviderKeysSectionProps) {
  const showToast = useToast();
  const { ask, dialog: confirmDialog } = useConfirmDialog();
  const queryClient = useQueryClient();
  const keysState = useQuery({
    queryKey: queryKeys.providerKeys(provider.id),
    queryFn: () => api.listProviderKeys(provider.id),
  });
  const [dialog, setDialog] = useState<{ isOpen: boolean; key: ProviderKeyDto | null }>({
    isOpen: false,
    key: null,
  });
  const [actionError, setActionError] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<ProviderTestResult | null>(null);
  const [isTesting, setIsTesting] = useState(false);
  const [syncMessage, setSyncMessage] = useState<string | null>(null);

  const keys = keysState.data ?? [];

  async function removeKeyNow(key: ProviderKeyDto) {
    setActionError(null);
    try {
      await api.deleteProviderKey(key.provider_id, key.id);
      void queryClient.invalidateQueries({
        queryKey: queryKeys.providerKeys(provider.id),
      });
      showToast({ body: "Key deleted", uniqueID: `delete-provider-key-${key.id}` });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  function removeKey(key: ProviderKeyDto) {
    ask({
      title: `Delete ${key.name}?`,
      description:
        'Requests stop using this credential immediately. This cannot be undone.',
      actionLabel: 'Delete key',
      onConfirm: () => removeKeyNow(key),
    });
  }

  async function testProvider() {
    setActionError(null);
    setTestResult(null);
    setIsTesting(true);
    try {
      setTestResult(await api.testProvider(provider.id));
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsTesting(false);
    }
  }

  return (
    <Section variant="transparent" dividers={['start']} paddingInlineStart={4}>
      {confirmDialog}
      <VStack gap={3}>
      <HStack gap={2} hAlign="between" vAlign="center" wrap="wrap">
        <Text type="supporting" size="sm">
          {keys.length === 1 ? '1 credential' : `${keys.length} credentials`} · pool for{' '}
          {provider.name}
        </Text>
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Button label="Test" variant="secondary" size="sm" onClick={testProvider} isLoading={isTesting} />
          <Button
            label="Add credential"
            variant="primary"
            size="sm"
            icon={<Icon icon={PlusIcon} />}
            onClick={() => setDialog({ isOpen: true, key: null })}
          />
        </HStack>
      </HStack>

      <ErrorBanner message={keysState.error?.message ?? actionError} />
      {testResult ? (
        <Banner
          status={testResult.ok ? 'success' : 'error'}
          title={testResult.ok ? 'Provider reachable' : 'Provider test failed'}
          description={formatTestResult(testResult)}
          isDismissable
          onDismiss={() => setTestResult(null)}
        />
      ) : null}
      {syncMessage ? (
        <Banner
          status="success"
          title="Models synced"
          description={syncMessage}
          isDismissable
          onDismiss={() => setSyncMessage(null)}
        />
      ) : null}

      <Table
        data={keys as KeyRow[]}
        idKey="id"
        density="spacious"
        dividers="rows"
        hasHover
        textOverflow="truncate"
        columns={[
          ...keyColumns,
          {
            key: 'actions',
            header: '',
            width: pixel(120),
            align: 'end',
            renderCell: (key) => (
              <HStack gap={2} justify="end">
                <IconButton
                  label={`Edit ${key.name}`}
                  tooltip={`Edit ${key.name}`}
                  variant="ghost"
                  size="sm"
                  icon={<Icon icon={PencilSquareIcon} />}
                  onClick={() => setDialog({ isOpen: true, key })}
                />
                <IconButton
                  label={`Delete ${key.name}`}
                  tooltip={`Delete ${key.name}`}
                  variant="ghost"
                  size="sm"
                  icon={<Icon icon={TrashIcon} />}
                  onClick={() => removeKey(key)}
                />
              </HStack>
            ),
          },
        ]}
        emptyState={<Text type="supporting">No credentials in this pool yet.</Text>}
      />
      <Text type="supporting" size="2xs">
        Updated {formatTime(provider.updated_at)}
      </Text>

      {dialog.isOpen ? (
        <ProviderKeyDialog
          key={dialog.key?.id ?? 'new-credential'}
          isOpen
          onOpenChange={(isOpen) => {
            if (!isOpen) setDialog({ isOpen: false, key: null });
          }}
          providerId={provider.id}
          providerKey={dialog.key}
          onSaved={() => {
            setDialog({ isOpen: false, key: null });
            void queryClient.invalidateQueries({
              queryKey: queryKeys.providerKeys(provider.id),
            });
          }}
        />
      ) : null}
    </VStack>
    </Section>
  );
}
