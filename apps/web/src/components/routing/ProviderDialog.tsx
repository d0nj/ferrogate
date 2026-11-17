import { useState } from 'react';

import { Button } from '@astryxdesign/core/Button';
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog';
import { Grid } from '@astryxdesign/core/Grid';
import { HStack } from '@astryxdesign/core/HStack';
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout';
import { NumberInput } from '@astryxdesign/core/NumberInput';
import { Selector } from '@astryxdesign/core/Selector';
import { Switch } from '@astryxdesign/core/Switch';
import { TextInput } from '@astryxdesign/core/TextInput';
import { VStack } from '@astryxdesign/core/VStack';

import { api } from '@/api/client';
import type { ApiDialect, ProviderDto, ProviderKind } from '@/api/types';
import { ErrorBanner } from '@/components/common/ErrorBanner';
import { MultiSelector } from '@astryxdesign/core/MultiSelector';

const PROVIDER_TYPES: ProviderKind[] = ['openai', 'anthropic', 'gemini', 'typesafe'];
const API_DIALECTS: ApiDialect[] = ['chat', 'responses'];

interface ProviderDialogProps {
  isOpen: boolean;
  onOpenChange: (isOpen: boolean) => void;
  provider: ProviderDto | null;
  onSaved: () => void;
}

export function ProviderDialog({ isOpen, onOpenChange, provider, onSaved }: ProviderDialogProps) {
  const [name, setName] = useState(provider?.name ?? '');
  const [providerType, setProviderType] = useState<string>(provider?.provider_type ?? 'openai');
  const [apiDialect, setApiDialect] = useState<string>(provider?.api_dialect ?? 'chat');
  const [baseUrl, setBaseUrl] = useState(provider?.base_url ?? '');
  const [apiKey, setApiKey] = useState('');
  const [models, setModels] = useState<string[]>(provider?.models ?? []);
  const [timeoutMs, setTimeoutMs] = useState<number | null>(provider?.timeout_ms ?? 30000);
  const [isActive, setIsActive] = useState(provider?.is_active ?? true);
  const [error, setError] = useState<string | null>(null);
  // Known models plus anything already stored on the provider, so an existing
  // selection stays visible even when it is absent from the catalogue.
  const modelOptions = [...new Set([...(provider?.models ?? []), ...models])];
  const isDialectSelectable = providerType === 'openai';
  const effectiveDialect = isDialectSelectable ? apiDialect : 'chat';

  async function save() {
    const trimmedName = name.trim();
    const trimmedUrl = baseUrl.trim();
    const parsedModels = models;

    if (!trimmedName || !trimmedUrl || parsedModels.length === 0) {
      setError('Name, base URL and at least one model are required.');
      return;
    }

    setError(null);
    try {
      if (provider) {
        await api.updateProvider(provider.id, {
          name: trimmedName,
          provider_type: providerType as ProviderKind,
          api_dialect: effectiveDialect as ApiDialect,
          base_url: trimmedUrl,
          api_key: apiKey.trim() ? apiKey.trim() : undefined,
          models: parsedModels,
          is_active: isActive,
          timeout_ms: timeoutMs ?? undefined,
        });
      } else {
        await api.createProvider({
          name: trimmedName,
          provider_type: providerType as ProviderKind,
          api_dialect: effectiveDialect as ApiDialect,
          base_url: trimmedUrl,
          api_key: apiKey.trim(),
          models: parsedModels,
          is_active: isActive,
          timeout_ms: timeoutMs ?? undefined,
        });
      }
      onSaved();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  return (
    <Dialog
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      purpose="form"
      width="min(580px, calc(100vw - 2rem))"
      maxHeight="85dvh"
    >
      <Layout
        header={
          <DialogHeader
            title={provider ? provider.name : 'New provider'}
            subtitle={provider ? provider.api_key_masked : undefined}
            onOpenChange={onOpenChange}
            hasDivider
          />
        }
        content={
          <LayoutContent>
            <VStack gap={3}>
              <ErrorBanner message={error} />
              <Grid columns={{ minWidth: 220, max: 2 }} gap={3}>
                <TextInput
                  label="Name"
                  value={name}
                  onChange={setName}
                  placeholder="openai-primary"
                />
                <Selector
                  label="Provider type"
                  options={PROVIDER_TYPES.map((type) => ({ value: type }))}
                  value={providerType}
                  onChange={setProviderType}
                />
              </Grid>
              <Selector
                label="API dialect"
                options={API_DIALECTS.map((dialect) => ({ value: dialect }))}
                value={effectiveDialect}
                onChange={setApiDialect}
                isDisabled={!isDialectSelectable}
                disabledMessage="Only the OpenAI provider type supports the responses dialect."
              />
              <TextInput
                label="Base URL"
                value={baseUrl}
                onChange={setBaseUrl}
                placeholder="https://api.openai.com/v1"
              />
              <TextInput
                label="API key"
                type="password"
                value={apiKey}
                onChange={setApiKey}
                placeholder={provider?.has_api_key ? 'Leave blank to keep current key' : 'sk-…'}
              />
              <Grid columns={{ minWidth: 220, max: 2 }} gap={3}>
                <MultiSelector
                  label="Models"
                  options={modelOptions}
                  value={models}
                  onChange={setModels}
                  placeholder="Select models"
                  hasSearch
                  hasCreate
                  hasClear
                />
                <NumberInput
                  label="Timeout (ms)"
                  value={timeoutMs}
                  min={1000}
                  step={1000}
                  isIntegerOnly
                  onChange={setTimeoutMs}
                />
              </Grid>
              <Switch label="Active" value={isActive} onChange={setIsActive} />
            </VStack>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider>
            <HStack gap={2} hAlign="end">
              <Button label="Cancel" variant="ghost" onClick={() => onOpenChange(false)} />
              <Button
                label={provider ? 'Save provider' : 'Create provider'}
                variant="primary"
                onClick={save}
              />
            </HStack>
          </LayoutFooter>
        }
      />
    </Dialog>
  );
}
