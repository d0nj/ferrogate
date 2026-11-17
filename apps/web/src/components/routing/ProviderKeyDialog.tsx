import { useState } from 'react';

import { Button } from '@astryxdesign/core/Button';
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog';
import { Grid } from '@astryxdesign/core/Grid';
import { HStack } from '@astryxdesign/core/HStack';
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout';
import { NumberInput } from '@astryxdesign/core/NumberInput';
import { Switch } from '@astryxdesign/core/Switch';
import { TextInput } from '@astryxdesign/core/TextInput';
import { VStack } from '@astryxdesign/core/VStack';

import { api } from '@/api/client';
import type { ProviderKeyDto } from '@/api/types';
import { ErrorBanner } from '@/components/common/ErrorBanner';
import { MultiSelector } from '@astryxdesign/core/MultiSelector';

interface ProviderKeyDialogProps {
  isOpen: boolean;
  onOpenChange: (isOpen: boolean) => void;
  providerId: string;
  providerKey: ProviderKeyDto | null;
  onSaved: () => void;
}

export function ProviderKeyDialog({
  isOpen,
  onOpenChange,
  providerId,
  providerKey,
  onSaved,
}: ProviderKeyDialogProps) {
  const [name, setName] = useState(providerKey?.name ?? '');
  const [apiKey, setApiKey] = useState('');
  const [weight, setWeight] = useState<number | null>(providerKey?.weight ?? 1);
  const [models, setModels] = useState<string[]>(providerKey?.models ?? []);
  // Blank means "use every model the provider offers", so nothing is preselected
  // for a new key; existing rows keep whatever they stored.
  const modelOptions = [...new Set([...(providerKey?.models ?? []), ...models])];
  const [rpm, setRpm] = useState<number | null>(providerKey?.rpm_limit ?? null);
  const [isActive, setIsActive] = useState(providerKey?.is_active ?? true);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    const trimmedName = name.trim();
    if (!trimmedName) {
      setError('Name is required.');
      return;
    }
    const trimmedKey = apiKey.trim();
    if (!providerKey && !trimmedKey) {
      setError('API key is required.');
      return;
    }

    setError(null);
    try {
      if (providerKey) {
        await api.updateProviderKey(providerKey.provider_id, providerKey.id, {
          name: trimmedName,
          api_key: trimmedKey || undefined,
          weight: weight ?? 1,
          models,
          rpm_limit: rpm ?? undefined,
          clear_rpm_limit: rpm === null,
          is_active: isActive,
        });
      } else {
        await api.createProviderKey(providerId, {
          name: trimmedName,
          api_key: trimmedKey,
          weight: weight ?? 1,
          models,
          rpm_limit: rpm,
          is_active: isActive,
        });
      }
      onSaved();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  async function clearCooldown() {
    if (!providerKey) return;
    setError(null);
    try {
      await api.updateProviderKey(providerKey.provider_id, providerKey.id, { clear_cooldown: true });
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
      width="min(540px, calc(100vw - 2rem))"
      maxHeight="85dvh"
    >
      <Layout
        header={
          <DialogHeader
            title={providerKey ? `Credential · ${providerKey.name}` : 'New credential'}
            onOpenChange={onOpenChange}
            hasDivider
          />
        }
        content={
          <LayoutContent>
            <VStack gap={3}>
              <ErrorBanner message={error} />
              <TextInput label="Name" value={name} onChange={setName} placeholder="primary" />
              <TextInput
                label={providerKey ? 'API key (leave blank to keep)' : 'API key'}
                type="password"
                value={apiKey}
                onChange={setApiKey}
                placeholder="sk-…"
              />
              <Grid columns={{ minWidth: 220, max: 2 }} gap={3}>
                <NumberInput
                  label="Weight"
                  value={weight}
                  min={1}
                  isIntegerOnly
                  onChange={setWeight}
                />
                <NumberInput
                  label="RPM limit"
                  value={rpm}
                  min={0}
                  hasClear
                  placeholder="Unlimited"
                  onChange={setRpm}
                />
              </Grid>
              <MultiSelector
                label="Models"
                options={modelOptions}
                value={models}
                onChange={setModels}
                placeholder="Inherit the provider catalogue"
                hasSearch
                hasCreate
                hasClear
              />
              <Switch label="Active" value={isActive} onChange={setIsActive} />
            </VStack>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider>
            <HStack gap={2} hAlign="end">
              {providerKey?.in_cooldown ? (
                <Button label="Clear cooldown" variant="secondary" onClick={clearCooldown} />
              ) : null}
              <Button label="Cancel" variant="ghost" onClick={() => onOpenChange(false)} />
              <Button
                label={providerKey ? 'Save credential' : 'Add credential'}
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
