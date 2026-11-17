import { useMemo, useState } from 'react';

import { Button } from '@astryxdesign/core/Button';
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog';
import { HStack, VStack } from '@astryxdesign/core/Stack';
import { Icon } from '@astryxdesign/core/Icon';
import { IconButton } from '@astryxdesign/core/IconButton';
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout';
import { MultiSelector } from '@astryxdesign/core/MultiSelector';
import { NumberInput } from '@astryxdesign/core/NumberInput';
import { Selector } from '@astryxdesign/core/Selector';
import type { SelectorOptionData } from '@astryxdesign/core/Selector';
import { Text } from '@astryxdesign/core/Text';
import { TextInput } from '@astryxdesign/core/TextInput';
import { Typeahead } from '@astryxdesign/core/Typeahead';
import type { SearchableItem } from '@astryxdesign/core/Typeahead';
import { ArrowDownIcon, ArrowUpIcon, PlusIcon, TrashIcon } from '@heroicons/react/24/outline';

import { api } from '@/api/client';
import type { ProviderDto, Route, RouteInput, RouteTargetInput, TierStrategy } from '@/api/types';
import { ErrorBanner } from '@/components/common/ErrorBanner';

const DEFAULT_WEIGHT = 100;
const WEIGHT_COLUMN = 132;

const STRATEGY_OPTIONS = [
  { value: 'weighted', label: 'weighted' },
  { value: 'latency', label: 'latency' },
  { value: 'cost', label: 'cost' },
  { value: 'priority', label: 'priority' },
];

interface RouteDialogProps {
  isOpen: boolean;
  onOpenChange: (isOpen: boolean) => void;
  route: Route | null;
  providers: ProviderDto[];
  aliases: string[];
  globalStrategy: TierStrategy | null;
  onSaved: () => void;
}

function toTargetInput(target: Route['targets'][number]): RouteTargetInput {
  return {
    provider_id: target.provider_id,
    upstream_model: target.upstream_model,
    weight: target.weight,
  };
}

function initialTargets(route: Route | null, providers: ProviderDto[]): RouteTargetInput[] {
  if (route) return route.targets.map(toTargetInput);
  return [{ provider_id: providers[0]?.id ?? '', upstream_model: '', weight: DEFAULT_WEIGHT }];
}

function moveTarget(targets: RouteTargetInput[], index: number, offset: number): RouteTargetInput[] {
  const destination = index + offset;
  if (destination < 0 || destination >= targets.length) return targets;
  const next = [...targets];
  const [moved] = next.splice(index, 1);
  next.splice(destination, 0, moved);
  return next;
}

function validate(alias: string, targets: RouteTargetInput[], fallbacks: string[]): string | null {
  if (!alias) return 'Alias is required.';
  if (targets.length === 0) return 'Add at least one target.';
  if (targets.some((target) => !target.provider_id)) return 'Every target needs a provider.';
  if (targets.some((target) => !target.upstream_model.trim())) {
    return 'Every target needs an upstream model.';
  }
  if (targets.some((target) => target.weight < 1)) return 'Weights must be at least 1.';
  if (fallbacks.includes(alias)) return 'A route cannot fall back to itself.';
  return null;
}

function providerOptions(providers: ProviderDto[]): SelectorOptionData[] {
  return providers.map((provider) => ({
    value: provider.id,
    label: provider.name,
    description: provider.provider_type,
  }));
}

/**
 * Upstream model for one target.
 *
 * The provider's declared models are offered as suggestions, but the field stays
 * free text: a provider with no synced model list (`models: []`) can still be
 * routed to by typing a name. Suggestions only remove the need to remember them.
 */
function ModelPicker({
  provider,
  value,
  label,
  isLabelHidden,
  onChange,
}: {
  provider: ProviderDto | undefined;
  value: string;
  label: string;
  isLabelHidden: boolean;
  onChange: (model: string) => void;
}) {
  const providerName = provider?.name;
  const declared = useMemo(() => provider?.models ?? [], [provider?.models]);

  const searchSource = useMemo(
    () => ({
      search(query: string): SearchableItem[] {
        const needle = query.trim().toLowerCase();
        const matches = declared
          .filter((model) => model.toLowerCase().includes(needle))
          .map((model) => ({ id: model, label: model }));
        // The typed name is always selectable, otherwise a model the provider
        // has not synced yet would be impossible to enter.
        const typed = query.trim();
        if (typed && !matches.some((item) => item.id === typed)) {
          matches.unshift({ id: typed, label: typed });
        }
        return matches;
      },
      bootstrap: () => declared.map((model) => ({ id: model, label: model })),
      cancel: () => undefined,
    }),
    [declared],
  );

  const selected: SearchableItem | null = value ? { id: value, label: value } : null;

  if (declared.length === 0) {
    return (
      <TextInput
        label={label}
        isLabelHidden={isLabelHidden}
        value={value}
        onChange={onChange}
        placeholder="gpt-4o-mini"
        description={providerName ? undefined : 'Pick a provider first'}
        width="100%"
      />
    );
  }

  return (
    <Typeahead
      label={label}
      isLabelHidden={isLabelHidden}
      value={selected}
      onChange={(item) => onChange(item?.id ?? '')}
      searchSource={searchSource}
      placeholder="Search this provider's models"
      hasEntriesOnFocus
      debounceMs={0}
      emptySearchText={null}
      width="100%"
    />
  );
}

interface TargetRowProps {
  target: RouteTargetInput;
  index: number;
  count: number;
  providers: ProviderDto[];
  options: SelectorOptionData[];
  showWeight: boolean;
  onChange: (index: number, patch: Partial<RouteTargetInput>) => void;
  onMove: (index: number, offset: number) => void;
  onRemove: (index: number) => void;
}

function TargetRow({
  target,
  index,
  count,
  providers,
  options,
  showWeight,
  onChange,
  onMove,
  onRemove,
}: TargetRowProps) {
  const position = index + 1;
  const provider = providers.find((candidate) => candidate.id === target.provider_id);

  return (
    <HStack gap={2} vAlign="end">
      <Selector
        label={`Target ${position}`}
        isLabelHidden={index > 0}
        options={options}
        value={target.provider_id}
        onChange={(providerId) => onChange(index, { provider_id: providerId })}
        hasSearch
        width="100%"
      />
      <ModelPicker
        provider={provider}
        label="Upstream model"
        isLabelHidden={index > 0}
        value={target.upstream_model}
        onChange={(upstreamModel) => onChange(index, { upstream_model: upstreamModel })}
      />
      {showWeight ? (
        <NumberInput
          label="Weight"
          isLabelHidden={index > 0}
          value={target.weight}
          min={1}
          isIntegerOnly
          onChange={(weight) => onChange(index, { weight })}
          width={WEIGHT_COLUMN}
        />
      ) : null}
      <IconButton
        label={`Move target ${position} up`}
        variant="ghost"
        size="sm"
        icon={<Icon icon={ArrowUpIcon} />}
        isDisabled={index === 0}
        onClick={() => onMove(index, -1)}
      />
      <IconButton
        label={`Move target ${position} down`}
        variant="ghost"
        size="sm"
        icon={<Icon icon={ArrowDownIcon} />}
        isDisabled={index === count - 1}
        onClick={() => onMove(index, 1)}
      />
      <IconButton
        label={`Remove target ${position}`}
        variant="ghost"
        size="sm"
        icon={<Icon icon={TrashIcon} />}
        isDisabled={count === 1}
        onClick={() => onRemove(index)}
      />
    </HStack>
  );
}

export function RouteDialog({
  isOpen,
  onOpenChange,
  route,
  providers,
  aliases,
  globalStrategy,
  onSaved,
}: RouteDialogProps) {
  const [alias, setAlias] = useState(route?.alias ?? '');
  const [strategy, setStrategy] = useState<TierStrategy | null>(route?.strategy ?? null);
  const [targets, setTargets] = useState<RouteTargetInput[]>(() => initialTargets(route, providers));
  const [fallbacks, setFallbacks] = useState<string[]>(route?.fallbacks ?? []);
  const [error, setError] = useState<string | null>(null);

  const showWeight = (strategy ?? globalStrategy) === 'weighted';
  const options = providerOptions(providers);
  const trimmedAlias = alias.trim();
  const fallbackOptions = aliases
    .filter((candidate) => candidate !== trimmedAlias)
    .map((candidate) => ({ value: candidate }));

  function changeTarget(index: number, patch: Partial<RouteTargetInput>) {
    setTargets((current) =>
      current.map((target, at) => (at === index ? { ...target, ...patch } : target)),
    );
  }

  function addTarget() {
    setTargets((current) => [
      ...current,
      { provider_id: providers[0]?.id ?? '', upstream_model: '', weight: DEFAULT_WEIGHT },
    ]);
  }

  async function save() {
    const body: RouteInput = {
      alias: trimmedAlias,
      strategy,
      fallbacks,
      targets: targets.map((target) => ({
        ...target,
        upstream_model: target.upstream_model.trim(),
      })),
    };

    const failure = validate(trimmedAlias, body.targets, fallbacks);
    if (failure) {
      setError(failure);
      return;
    }

    setError(null);
    try {
      await api.putRoute(trimmedAlias, body);
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
      width="min(680px, calc(100vw - 2rem))"
      maxHeight="85dvh"
    >
      <Layout
        header={
          <DialogHeader
            title={route ? `Route · ${route.alias}` : 'New route'}
            onOpenChange={onOpenChange}
            hasDivider
          />
        }
        content={
          <LayoutContent>
            <VStack gap={4}>
              <ErrorBanner message={error} />
              <HStack gap={3} vAlign="end">
                <TextInput
                  label="Alias"
                  value={alias}
                  onChange={setAlias}
                  placeholder="smart"
                  isRequired
                  width="100%"
                />
                <Selector
                  label="Strategy"
                  options={STRATEGY_OPTIONS}
                  value={strategy}
                  onChange={(value) => setStrategy(value as TierStrategy | null)}
                  placeholder="Use global policy"
                  hasClear
                  width={220}
                />
              </HStack>

              <VStack gap={2}>
                <HStack gap={2} vAlign="center" justify="between">
                  <VStack gap={0}>
                    <Text weight="semibold">Targets</Text>
                    <Text type="supporting" size="sm">
                      Tried top to bottom.
                    </Text>
                  </VStack>
                  <Button
                    label="Add target"
                    variant="secondary"
                    size="sm"
                    icon={<Icon icon={PlusIcon} />}
                    onClick={addTarget}
                  />
                </HStack>
                {targets.map((target, index) => (
                  <TargetRow
                    key={`target-${index}`}
                    target={target}
                    index={index}
                    count={targets.length}
                    providers={providers}
                    options={options}
                    showWeight={showWeight}
                    onChange={changeTarget}
                    onMove={(at, offset) =>
                      setTargets((current) => moveTarget(current, at, offset))
                    }
                    onRemove={(at) =>
                      setTargets((current) => current.filter((_, position) => position !== at))
                    }
                  />
                ))}
              </VStack>

              <MultiSelector
                label="Fallbacks"
                description="Other aliases tried in order when every target above fails."
                options={fallbackOptions}
                value={fallbacks}
                onChange={setFallbacks}
                placeholder="No fallbacks"
                hasSearch
              />
            </VStack>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider>
            <HStack gap={2} hAlign="end">
              <Button label="Cancel" variant="ghost" onClick={() => onOpenChange(false)} />
              <Button label={route ? 'Save route' : 'Create route'} variant="primary" onClick={save} />
            </HStack>
          </LayoutFooter>
        }
      />
    </Dialog>
  );
}
