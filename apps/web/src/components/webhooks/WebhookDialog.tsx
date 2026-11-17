import { useState } from 'react';

import { Button } from '@astryxdesign/core/Button';
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog';
import { HStack } from '@astryxdesign/core/HStack';
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout';
import { MultiSelector } from '@astryxdesign/core/MultiSelector';
import { Switch } from '@astryxdesign/core/Switch';
import { TextInput } from '@astryxdesign/core/TextInput';
import { VStack } from '@astryxdesign/core/VStack';

import { api } from '@/api/client';
import type { WebhookDto } from '@/api/types';
import { ErrorBanner } from '@/components/common/ErrorBanner';

const EVENT_PRESETS = ['ping', 'request.blocked', 'budget.exceeded', 'guardrail.triggered'];

interface WebhookDialogProps {
  isOpen: boolean;
  onOpenChange: (isOpen: boolean) => void;
  webhook: WebhookDto | null;
  onSaved: () => void;
}

export function WebhookDialog({ isOpen, onOpenChange, webhook, onSaved }: WebhookDialogProps) {
  const [name, setName] = useState(webhook?.name ?? '');
  const [url, setUrl] = useState(webhook?.url ?? '');
  const [secret, setSecret] = useState('');
  const [events, setEvents] = useState<string[]>(webhook?.events ?? ['ping']);
  const [isActive, setIsActive] = useState(webhook?.is_active ?? true);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    const trimmedName = name.trim();
    const trimmedUrl = url.trim();
    if (!trimmedName || !trimmedUrl) {
      setError('Name and URL are required.');
      return;
    }
    if (events.length === 0) {
      setError('Select at least one event.');
      return;
    }

    setError(null);
    try {
      const trimmedSecret = secret.trim();
      if (webhook) {
        await api.updateWebhook(webhook.id, {
          name: trimmedName,
          url: trimmedUrl,
          ...(trimmedSecret ? { secret: trimmedSecret } : {}),
          events,
          is_active: isActive,
        });
      } else {
        await api.createWebhook({
          name: trimmedName,
          url: trimmedUrl,
          secret: trimmedSecret || null,
          events,
          is_active: isActive,
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
      width="min(540px, calc(100vw - 2rem))"
      maxHeight="85dvh"
    >
      <Layout
        header={
          <DialogHeader
            title={webhook ? `Webhook · ${webhook.name}` : 'New webhook'}
            onOpenChange={onOpenChange}
            hasDivider
          />
        }
        content={
          <LayoutContent>
            <VStack gap={3}>
              <ErrorBanner message={error} />
              <TextInput
                label="Name"
                value={name}
                onChange={setName}
                placeholder="ops-alerts"
              />
              <TextInput
                label="URL"
                value={url}
                onChange={setUrl}
                placeholder="https://ops.example.com/hooks/ferrogate"
              />
              <TextInput
                label={
                  webhook
                    ? `Secret (stored: ${webhook.has_secret ? webhook.secret_masked : 'none'})`
                    : 'Secret'
                }
                type="password"
                value={secret}
                onChange={setSecret}
                placeholder={webhook ? 'Leave blank to keep' : 'Optional signing secret'}
              />
              <MultiSelector
                label="Events"
                options={[...new Set([...EVENT_PRESETS, ...events])].map((value) => ({ value }))}
                value={events}
                onChange={setEvents}
                placeholder="Select events"
                hasSearch
                hasCreate
                hasSelectAll
              />
              <Switch label="Active" value={isActive} onChange={setIsActive} />
            </VStack>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider>
            <HStack gap={2} hAlign="end" width="100%">
              <Button label="Cancel" variant="ghost" onClick={() => onOpenChange(false)} />
              <Button
                label={webhook ? 'Save webhook' : 'Create webhook'}
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
