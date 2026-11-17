import { useState } from 'react';

import type { ISODateString } from '@astryxdesign/core/Calendar';
import { Banner } from '@astryxdesign/core/Banner';
import { Button } from '@astryxdesign/core/Button';
import { CodeBlock } from '@astryxdesign/core/CodeBlock';
import { DateInput } from '@astryxdesign/core/DateInput';
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog';
import { Grid } from '@astryxdesign/core/Grid';
import { HStack } from '@astryxdesign/core/HStack';
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout';
import { MultiSelector } from '@astryxdesign/core/MultiSelector';
import { NumberInput } from '@astryxdesign/core/NumberInput';
import { TextInput } from '@astryxdesign/core/TextInput';
import { VStack } from '@astryxdesign/core/VStack';

import { api } from '@/api/client';
import type { CreateKeyResponse } from '@/api/types';
import { ErrorBanner } from '@/components/common/ErrorBanner';

interface CreateKeyDialogProps {
  isOpen: boolean;
  onOpenChange: (isOpen: boolean) => void;
  models: string[];
  allTags: string[];
  allMcp: string[];
  onCreated: () => void;
}

export function CreateKeyDialog({
  isOpen,
  onOpenChange,
  models,
  allTags,
  allMcp,
  onCreated,
}: CreateKeyDialogProps) {
  const [name, setName] = useState('');
  const [budget, setBudget] = useState<number | null>(null);
  const [rpm, setRpm] = useState<number | null>(null);
  const [tpm, setTpm] = useState<number | null>(null);
  const [allowedModels, setAllowedModels] = useState<string[]>([]);
  const [expires, setExpires] = useState<ISODateString | undefined>(undefined);
  const [team, setTeam] = useState('');
  const [tags, setTags] = useState<string[]>([]);
  const [allowedMcp, setAllowedMcp] = useState<string[]>([]);
  const [created, setCreated] = useState<CreateKeyResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  const close = (next: boolean) => {
    if (next) return;
    setName('');
    setBudget(null);
    setRpm(null);
    setTpm(null);
    setAllowedModels([]);
    setExpires(undefined);
    setTeam('');
    setTags([]);
    setAllowedMcp([]);
    setCreated(null);
    setError(null);
    onOpenChange(false);
  };

  async function submit() {
    if (!name.trim()) {
      setError('Name is required.');
      return;
    }

    setError(null);
    try {
      const response = await api.createKey({
        name: name.trim(),
        budget_usd: budget,
        rpm_limit: rpm,
        tpm_limit: tpm,
        allowed_models: allowedModels,
        expires_at: expires ? `${expires}T00:00:00Z` : undefined,
        team_name: team.trim(),
        tags,
        allowed_mcp: allowedMcp,
      });
      setCreated(response);
      onCreated();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  if (created) {
    return (
      <Dialog
        isOpen={isOpen}
        onOpenChange={close}
        purpose="form"
        width="min(620px, calc(100vw - 2rem))"
        maxHeight="85dvh">
        <Layout
          header={
            <DialogHeader
              title="Virtual key created"
              onOpenChange={close}
              hasDivider
            />
          }
          content={
            <LayoutContent>
              <VStack gap={3}>
                <Banner
                  status="warning"
                  title="Copy this key now"
                  description="The full key is shown once and cannot be retrieved later."
                />
                <CodeBlock
                  code={created.plaintext_key}
                  language="text"
                  hasCopyButton
                  isWrapped
                />
              </VStack>
            </LayoutContent>
          }
          footer={
            <LayoutFooter hasDivider>
              <HStack gap={2} hAlign="end" width="100%">
                <Button label="Done" variant="primary" onClick={() => close(false)} />
              </HStack>
            </LayoutFooter>
          }
        />
      </Dialog>
    );
  }

  return (
    <Dialog
      isOpen={isOpen}
      onOpenChange={close}
      purpose="form"
      width="min(560px, calc(100vw - 2rem))"
      maxHeight="85dvh">
      <Layout
        header={
          <DialogHeader
            title="New virtual key"
            onOpenChange={close}
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
                placeholder="production-api"
              />
              <Grid columns={{ minWidth: 220, max: 2 }} gap={3}>
                <TextInput
                  label="Team"
                  value={team}
                  onChange={setTeam}
                  placeholder="platform"
                  isOptional
                />
                <DateInput
                  label="Expires"
                  value={expires}
                  onChange={setExpires}
                  hasClear
                  isOptional
                />
              </Grid>
              <NumberInput
                label="Budget (USD)"
                value={budget}
                min={0}
                step={0.5}
                placeholder="Unlimited"
                onChange={setBudget}
              />
              <Grid columns={{ minWidth: 220, max: 2 }} gap={3}>
                <NumberInput
                  label="RPM limit"
                  value={rpm}
                  min={0}
                  placeholder="Unlimited"
                  onChange={setRpm}
                />
                <NumberInput
                  label="TPM limit"
                  value={tpm}
                  min={0}
                  placeholder="Unlimited"
                  onChange={setTpm}
                />
              </Grid>
              <MultiSelector
                label="Allowed models"
                options={models.map((model) => ({ value: model }))}
                value={allowedModels}
                onChange={setAllowedModels}
                placeholder="All models"
                hasSearch
              />
              <MultiSelector
                label="Tags"
                options={[...new Set([...allTags, ...tags])].map((tag) => ({ value: tag }))}
                value={tags}
                onChange={setTags}
                placeholder="No tags"
                hasSearch
                hasCreate
              />
              <MultiSelector
                label="Allowed MCP servers"
                options={[...new Set([...allMcp, ...allowedMcp])].map((server) => ({
                  value: server,
                }))}
                value={allowedMcp}
                onChange={setAllowedMcp}
                placeholder="All servers"
                hasSearch
                hasCreate
              />
            </VStack>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider>
            <HStack gap={2} hAlign="end" width="100%">
              <Button label="Cancel" variant="ghost" onClick={() => close(false)} />
              <Button label="Create key" variant="primary" onClick={submit} />
            </HStack>
          </LayoutFooter>
        }
      />
    </Dialog>
  );
}
