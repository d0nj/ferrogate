import { useState } from "react";

import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { HStack } from "@astryxdesign/core/HStack";
import { Icon } from "@astryxdesign/core/Icon";
import { NumberInput } from "@astryxdesign/core/NumberInput";
import { Selector } from "@astryxdesign/core/Selector";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Heading, Text } from "@astryxdesign/core/Text";
import { Section } from "@astryxdesign/core/Section";
import { VStack } from "@astryxdesign/core/VStack";
import { ArrowPathIcon, TrashIcon } from "@heroicons/react/24/outline";

import { api } from "@/api/client";
import { queryKeys } from "@/api/queryKeys";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import type { ProviderDto } from "@/api/types";
import { formatNumber } from "@/utils/format";
import { useQuery, useQueryClient } from "@tanstack/react-query";

/// Models offered for embeddings: every model on an active provider, plus
/// whatever is already selected so a stored value never disappears from the list.
function embeddingOptions(
  providers: ProviderDto[],
  selected: string | null | undefined,
) {
  const preferred = new Map<string, string>();
  const rest = new Map<string, string>();

  for (const provider of providers) {
    for (const model of provider.models) {
      const target = /embed/i.test(model) ? preferred : rest;
      if (!target.has(model)) target.set(model, provider.name);
    }
  }

  // Embedding-shaped ids first; everything else stays selectable because the
  // proxy routes by name and an unfamiliar id may still serve embeddings.
  const seen = preferred.size > 0 ? preferred : rest;
  if (selected && !seen.has(selected)) seen.set(selected, 'current');

  return [...seen].map(([value, provider]) => ({
    value,
    label: value,
    description: provider === 'current' ? 'Currently selected' : provider,
  }));
}

function roundTwo(value: number): number {
  return Math.round(value * 100) / 100;
}

export function SemanticCachePanel() {
  const queryClient = useQueryClient();
  const statsState = useQuery({
    queryKey: queryKeys.semanticStats,
    queryFn: api.getSemanticStats,
  });
  const providersState = useQuery({
    queryKey: queryKeys.providers,
    queryFn: api.listProviders,
  });
  const { data, isLoading } = statsState;
  const error = statsState.error?.message ?? null;
  const [editedThreshold, setEditedThreshold] = useState<
    number | null | undefined
  >(undefined);
  const threshold =
    editedThreshold === undefined
      ? data
        ? roundTwo(data.threshold)
        : null
      : editedThreshold;
  const [actionError, setActionError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [isClearing, setIsClearing] = useState(false);
  const [clearedAt, setClearedAt] = useState<string | null>(null);
  const [isSavingModel, setIsSavingModel] = useState(false);

  async function saveThreshold() {
    setActionError(null);
    if (
      threshold === null ||
      Number.isNaN(threshold) ||
      threshold < 0.5 ||
      threshold > 1
    ) {
      setActionError("Threshold must be between 0.50 and 1.00.");
      return;
    }
    setIsSaving(true);
    try {
      const updated = await api.putSemanticThreshold(roundTwo(threshold));
      setEditedThreshold(roundTwo(updated.threshold));
      void queryClient.invalidateQueries({ queryKey: queryKeys.semanticStats });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsSaving(false);
    }
  }

  async function saveEmbeddingModel(next: string | null) {
    setActionError(null);
    setIsSavingModel(true);
    try {
      await api.putSemanticEmbeddingModel(next);
      void queryClient.invalidateQueries({ queryKey: queryKeys.semanticStats });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsSavingModel(false);
    }
  }

  async function clearSemantic() {
    setActionError(null);
    setClearedAt(null);
    setIsClearing(true);
    try {
      await api.clearSemanticCache();
      setClearedAt(new Date().toLocaleTimeString());
      void queryClient.invalidateQueries({ queryKey: queryKeys.semanticStats });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsClearing(false);
    }
  }

  const stats = data;
  // The backend refuses every mutation without QDRANT_URL, so the controls must
  // not offer them; the panel explains why instead of silently failing.
  const isEnabled = stats?.qdrant_configured ?? false;

  return (
    <Section padding={4}>
      <VStack gap={3}>
        <HStack hAlign="between" vAlign="center" gap={4} wrap="wrap">
          <VStack gap={0.5}>
            <Heading level={3}>Semantic cache</Heading>
            <Text type="supporting" size="sm">
              Embedding similarity matches served without an upstream call.
              {clearedAt ? ` Cleared at ${clearedAt}.` : ""}
            </Text>
          </VStack>
          <HStack gap={2} vAlign="center">
            <Button
              label="Refresh"
              variant="ghost"
              size="sm"
              icon={<Icon icon={ArrowPathIcon} />}
              onClick={() => void statsState.refetch()}
              isLoading={isLoading}
            />
            <Button
              label="Clear semantic cache"
              variant="secondary"
              size="sm"
              icon={<Icon icon={TrashIcon} />}
              onClick={clearSemantic}
              isDisabled={!isEnabled}
              isLoading={isClearing}
            />
          </HStack>
        </HStack>
        <ErrorBanner message={error ?? actionError} />
        <VStack gap={2}>
          <HStack gap={2} vAlign="end">
            <NumberInput
              label="Similarity threshold"
              value={threshold}
              min={0.5}
              max={1}
              step={0.01}
              onChange={setEditedThreshold}
              isDisabled={isLoading || isSaving || !isEnabled}
            />
            <Button
              label="Save threshold"
              variant="primary"
              size="sm"
              onClick={saveThreshold}
              isLoading={isSaving}
              isDisabled={threshold === null || !isEnabled}
            />
          </HStack>
          <Text type="supporting" size="sm">
            Minimum cosine similarity for a semantic hit.
          </Text>
        </VStack>
        <VStack gap={2}>
          <Selector
            label="Embedding model"
            options={embeddingOptions(providersState.data ?? [], stats?.embedding_model)}
            value={stats?.embedding_model ?? null}
            onChange={(next) => void saveEmbeddingModel(next ?? null)}
            hasClear
            hasSearch
            placeholder={isSavingModel ? "Saving…" : "Select an embedding model"}
            isDisabled={isLoading || isSavingModel || !isEnabled}
          />
          <Text type="supporting" size="sm">
            Routed through this gateway like any other embeddings call. A model
            is required: the built-in fallback has been removed.
          </Text>
        </VStack>
        <Table
          data={[
            {
              metric: "Cached entries",
              value: stats ? formatNumber(stats.entries) : "—",
            },
            {
              metric: "Semantic hits",
              value: stats ? formatNumber(stats.hits) : "—",
            },
            {
              metric: "Backend",
              value: !stats
                ? "—"
                : stats.qdrant_configured
                  ? stats.backend
                  : "sqlite (Qdrant disabled: QDRANT_URL is not set)",
            },
          ]}
          density="spacious"
          dividers="rows"
          columns={[
            {
              key: "metric",
              header: "Metric",
              width: proportional(1, { minWidth: 140 }),
              renderCell: (row) => (
                <Text weight="semibold">{String(row.metric)}</Text>
              ),
            },
            {
              key: "value",
              header: "Value",
              align: "end",
              width: pixel(160),
              renderCell: (row) =>
                row.metric === "Backend" && row.value !== "—" ? (
                  <Badge variant="neutral" label={String(row.value)} />
                ) : (
                  <Text size="sm" hasTabularNumbers>
                    {String(row.value)}
                  </Text>
                ),
            },
          ]}
          emptyState={null}
        />
      </VStack>
    </Section>
  );
}
