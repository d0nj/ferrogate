import { useState } from 'react';

import { Button } from '@astryxdesign/core/Button';
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog';
import { Grid } from '@astryxdesign/core/Grid';
import { HStack } from '@astryxdesign/core/HStack';
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout';
import { NumberInput } from '@astryxdesign/core/NumberInput';
import { Text } from '@astryxdesign/core/Text';
import { TextArea } from '@astryxdesign/core/TextArea';
import { TextInput } from '@astryxdesign/core/TextInput';
import { VStack } from '@astryxdesign/core/VStack';

import { api } from '@/api/client';
import type { PromptVersionDto } from '@/api/types';
import { ErrorBanner } from '@/components/common/ErrorBanner';
import { formatPromptVariables, parsePromptVariables } from './constants';

function isValidSlug(slug: string): boolean {
  if (slug.length === 0) return false;
  return [...slug].every(
    (char) =>
      (char >= 'a' && char <= 'z') ||
      (char >= 'A' && char <= 'Z') ||
      (char >= '0' && char <= '9') ||
      char === '-' ||
      char === '_' ||
      char === '.',
  );
}

function deriveSlug(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

interface PromptDialogProps {
  isOpen: boolean;
  onOpenChange: (isOpen: boolean) => void;
  mode: 'create' | 'edit' | 'version';
  prompt: PromptVersionDto | null;
  onSaved: () => void;
}

export function PromptDialog({ isOpen, onOpenChange, mode, prompt, onSaved }: PromptDialogProps) {
  const [name, setName] = useState(prompt?.name ?? '');
  const [systemPrompt, setSystemPrompt] = useState(prompt?.system_prompt ?? '');
  const [userTemplate, setUserTemplate] = useState(prompt?.user_template ?? '');
  const [variablesText, setVariablesText] = useState(formatPromptVariables(prompt?.variables ?? []));
  const [model, setModel] = useState(prompt?.model ?? '');
  const [temperature, setTemperature] = useState<number | null>(prompt?.temperature ?? null);
  const [maxTokens, setMaxTokens] = useState<number | null>(prompt?.max_tokens ?? null);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    const trimmedName = name.trim();
    if (!trimmedName) {
      setError('Name is required.');
      return;
    }
    if (mode === 'create' && !isValidSlug(deriveSlug(trimmedName))) {
      setError('Name is required.');
      return;
    }

    setError(null);
    try {
      const trimmedModel = model.trim();
      const body = {
        name: trimmedName,
        system_prompt: systemPrompt,
        user_template: userTemplate,
        model: trimmedModel || null,
        temperature,
        max_tokens: maxTokens,
        variables: parsePromptVariables(variablesText),
      };
      if (mode === 'edit' && prompt) {
        await api.updatePrompt(prompt.slug, prompt.version, body);
      } else if (prompt) {
        await api.createPrompt({ slug: prompt.slug, ...body });
      } else {
        await api.createPrompt({ slug: deriveSlug(trimmedName), ...body });
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
            title={
              mode === 'create'
                ? 'New prompt'
                : mode === 'version'
                  ? `Prompt · ${prompt?.slug} · new version`
                  : `Prompt · ${prompt?.slug} v${prompt?.version}`
            }
            onOpenChange={onOpenChange}
            hasDivider
          />
        }
        content={
          <LayoutContent>
            <VStack gap={3}>
              <ErrorBanner message={error} />
              <VStack gap={1}>
                <TextInput label="Name" value={name} onChange={setName} placeholder="summarizer" />
                {mode === 'create' ? (
                  <Text type="supporting" size="sm">
                    Slug · {deriveSlug(name)}
                  </Text>
                ) : null}
              </VStack>
              <TextArea
                label="System prompt"
                value={systemPrompt}
                onChange={setSystemPrompt}
                placeholder="You are a helpful assistant."
              />
              <TextArea
                label="User template"
                value={userTemplate}
                onChange={setUserTemplate}
                placeholder="Summarize this: {{ topic }}"
                description="Use {{ variable }} placeholders for runtime values."
              />
              <TextArea
                label="Variables"
                value={variablesText}
                onChange={setVariablesText}
                placeholder={'topic\ntone=concise'}
                description="One per line: name or name=default."
              />
              <TextInput
                label="Model"
                value={model}
                onChange={setModel}
                placeholder="Optional override, blank clears"
              />
              <Grid columns={{ minWidth: 220, max: 2 }} gap={3}>
                <NumberInput
                  label="Temperature"
                  value={temperature}
                  min={0}
                  step={0.1}
                  description="Blank for default."
                  onChange={setTemperature}
                />
                <NumberInput
                  label="Max tokens"
                  value={maxTokens}
                  min={1}
                  isIntegerOnly
                  description="Blank for default."
                  onChange={setMaxTokens}
                />
              </Grid>
            </VStack>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider>
            <HStack gap={2} hAlign="end" width="100%">
              <Button label="Cancel" variant="ghost" onClick={() => onOpenChange(false)} />
              <Button
                label={mode === 'edit' ? 'Save prompt' : mode === 'version' ? 'Create version' : 'Create prompt'}
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
