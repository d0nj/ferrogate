import { useState } from 'react';

import type { ISODateString } from '@astryxdesign/core/Calendar';
import { Button } from '@astryxdesign/core/Button';
import { DateInput } from '@astryxdesign/core/DateInput';
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog';
import { Grid } from '@astryxdesign/core/Grid';
import { HStack } from '@astryxdesign/core/HStack';
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout';
import { MultiSelector } from '@astryxdesign/core/MultiSelector';
import { NumberInput } from '@astryxdesign/core/NumberInput';
import { Switch } from '@astryxdesign/core/Switch';
import { TextInput } from '@astryxdesign/core/TextInput';
import { VStack } from '@astryxdesign/core/VStack';

import { api } from '@/api/client';
import type { UpdateKeyInput, VirtualKeyDto } from '@/api/types';
import { ErrorBanner } from '@/components/common/ErrorBanner';

function toDateInput(value: string | null): ISODateString | undefined {
  if (!value || value.length < 10) return undefined;
  return value.slice(0, 10) as ISODateString;
}

interface EditKeyDialogProps {
  isOpen: boolean;
  onOpenChange: (isOpen: boolean) => void;
  virtualKey: VirtualKeyDto;
  models: string[];
  allTags: string[];
  allMcp: string[];
  onSaved: () => void;
}

export function EditKeyDialog({
  isOpen,
  onOpenChange,
  virtualKey,
  models,
  allTags,
  allMcp,
  onSaved,
}: EditKeyDialogProps) {
  const [name, setName] = useState(virtualKey.name);
  const [budget, setBudget] = useState<number | null>(virtualKey.budget_usd);
  const [rpm, setRpm] = useState<number | null>(virtualKey.rpm_limit);
  const [tpm, setTpm] = useState<number | null>(virtualKey.tpm_limit);
  const [allowedModels, setAllowedModels] = useState<string[]>(virtualKey.allowed_models);
  const [expires, setExpires] = useState<ISODateString | undefined>(
    toDateInput(virtualKey.expires_at),
  );
  const [team, setTeam] = useState(virtualKey.team_name ?? '');
  const [tags, setTags] = useState<string[]>(virtualKey.tags);
  const [allowedMcp, setAllowedMcp] = useState<string[]>(virtualKey.allowed_mcp);
  const [isActive, setIsActive] = useState(virtualKey.is_active);
  const [error, setError] = useState<string | null>(null);

  async function patch(body: UpdateKeyInput) {
    setError(null);
    try {
      await api.updateKey(virtualKey.id, body);
      onSaved();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  function save() {
    const initialExpires = toDateInput(virtualKey.expires_at);
    void patch({
      name: name.trim() || virtualKey.name,
      budget_usd: budget ?? undefined,
      clear_budget: budget === null,
      rpm_limit: rpm ?? undefined,
      clear_rpm_limit: rpm === null,
      tpm_limit: tpm ?? undefined,
      clear_tpm_limit: tpm === null,
      allowed_models: allowedModels,
      expires_at: expires && expires !== initialExpires ? `${expires}T00:00:00Z` : undefined,
      clear_expires_at: !expires && initialExpires !== undefined,
      team_name: team.trim(),
      tags,
      allowed_mcp: allowedMcp,
      is_active: isActive,
    });
  }

  return (
    <Dialog
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      purpose="form"
      width="min(560px, calc(100vw - 2rem))"
      maxHeight="85dvh">
      <Layout
        header={
          <DialogHeader
            title={virtualKey.name}
            subtitle={virtualKey.key_prefix}
            onOpenChange={onOpenChange}
            hasDivider
          />
        }
        content={
          <LayoutContent>
            <VStack gap={3}>
              <ErrorBanner message={error} />
              <TextInput label="Name" value={name} onChange={setName} />
              <Grid columns={{ minWidth: 220, max: 2 }} gap={3}>
                <TextInput
                  label="Team"
                  value={team}
                  onChange={setTeam}
                  placeholder="No team"
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
                hasClear
                placeholder="Unlimited"
                onChange={setBudget}
              />
              <Grid columns={{ minWidth: 220, max: 2 }} gap={3}>
                <NumberInput
                  label="RPM limit"
                  value={rpm}
                  min={0}
                  hasClear
                  placeholder="Unlimited"
                  onChange={setRpm}
                />
                <NumberInput
                  label="TPM limit"
                  value={tpm}
                  min={0}
                  hasClear
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
              <Switch label="Active" value={isActive} onChange={setIsActive} />
            </VStack>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider>
            <HStack gap={2} hAlign="between" vAlign="center" wrap="wrap">
              <Button
                label="Reset spend"
                variant="secondary"
                onClick={() => patch({ reset_spend: true })}
              />
              <HStack gap={2}>
                <Button label="Cancel" variant="ghost" onClick={() => onOpenChange(false)} />
                <Button label="Save key" variant="primary" onClick={save} />
              </HStack>
            </HStack>
          </LayoutFooter>
        }
      />
    </Dialog>
  );
}
