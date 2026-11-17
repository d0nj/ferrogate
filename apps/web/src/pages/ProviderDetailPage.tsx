import { useState } from "react";

import { useParams, useRouter } from "@tanstack/react-router";

import { Badge } from "@astryxdesign/core/Badge";
import { Breadcrumbs, BreadcrumbItem } from "@astryxdesign/core/Breadcrumbs";
import { Button } from "@astryxdesign/core/Button";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { Icon } from "@astryxdesign/core/Icon";
import { HStack } from "@astryxdesign/core/HStack";
import { Section } from "@astryxdesign/core/Section";
import { StackItem } from "@astryxdesign/core/Stack";
import { Spinner } from "@astryxdesign/core/Spinner";
import { Card } from "@astryxdesign/core/Card";
import { Grid } from "@astryxdesign/core/Grid";
import { Item } from "@astryxdesign/core/Item";
import { TextInput } from "@astryxdesign/core/TextInput";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";

import { api } from "@/api/client";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { ProviderKeysSection } from "@/components/routing/ProviderKeysSection";
import { ProviderDialog } from "@/components/routing/ProviderDialog";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { TrashIcon } from "@heroicons/react/24/outline";
import { queryKeys } from "@/api/queryKeys";
import { useToast } from "@astryxdesign/core/Toast";

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <VStack gap={0.5}>
      <Text type="supporting" size="2xs">
        {label}
      </Text>
      {children}
    </VStack>
  );
}

/**
 * Four grid rows (44px cells + 8px gaps) so the cap lands on a row boundary
 * instead of slicing a cell in half.
 */
const MODEL_CELL_HEIGHT = 44;
const MODEL_CELL_GAP = 8;
const MODEL_LIST_ROWS = 4;
const MODEL_LIST_MAX_HEIGHT =
  MODEL_CELL_HEIGHT * MODEL_LIST_ROWS + MODEL_CELL_GAP * (MODEL_LIST_ROWS - 1);

export function ProviderDetailPage() {
  const { providerId } = useParams({ from: "/routing/$providerId" });
  const router = useRouter();
  const showToast = useToast();
  const { ask, dialog } = useConfirmDialog();

  const queryClient = useQueryClient();

  const providersState = useQuery({
    queryKey: queryKeys.providers,
    queryFn: api.listProviders,
  });
  const provider = (providersState.data ?? []).find((row) => row.id === providerId);

  const [isEditing, setIsEditing] = useState(false);
  const [isSyncing, setIsSyncing] = useState(false);
  const [newModel, setNewModel] = useState("");
  const [modelFilter, setModelFilter] = useState("");
  const [actionError, setActionError] = useState<string | null>(null);

  // Large catalogues (OpenRouter, NVIDIA NIM) run to 500+ models; filter before capping.
  const allModels = provider?.models ?? [];
  const needle = modelFilter.trim().toLowerCase();
  const visibleModels = needle
    ? allModels.filter((model) => model.toLowerCase().includes(needle))
    : allModels;

  function reload() {
    void queryClient.invalidateQueries({ queryKey: queryKeys.providers });
  }

  async function syncModels() {
    if (!provider) return;
    setIsSyncing(true);
    setActionError(null);
    try {
      const result = await api.syncProviderModels(provider.id);
      reload();
      showToast({
        body:
          result.removed_count > 0
            ? `Synced: ${result.added_count} added, ${result.removed_count} removed`
            : `Synced: ${result.added_count} added`,
        uniqueID: `sync-models-${provider.id}`,
      });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsSyncing(false);
    }
  }

  async function removeModel(model: string) {
    if (!provider) return;
    setActionError(null);
    try {
      await api.updateProvider(provider.id, {
        models: provider.models.filter((known) => known !== model),
      });
      reload();
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  async function addModel() {
    if (!provider) return;
    const trimmed = newModel.trim();
    if (!trimmed || provider.models.includes(trimmed)) return;
    setActionError(null);
    try {
      await api.updateProvider(provider.id, {
        models: [...provider.models, trimmed].sort(),
      });
      setNewModel("");
      reload();
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  async function deleteProviderNow() {
    if (!provider) return;
    setActionError(null);
    try {
      await api.deleteProvider(provider.id);
      showToast({ body: "Provider deleted", uniqueID: `delete-provider-${provider.id}` });
      await router.navigate({ to: "/routing" });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  if (providersState.isLoading) {
    return (
      <VStack padding={6} align="center">
        <Spinner label="Loading provider" />
      </VStack>
    );
  }

  if (!provider) {
    return (
      <VStack gap={4}>
        <Breadcrumbs>
          <BreadcrumbItem href="/routing">Routing</BreadcrumbItem>
          <BreadcrumbItem>Not found</BreadcrumbItem>
        </Breadcrumbs>
        <EmptyState title="This provider no longer exists." />
      </VStack>
    );
  }

  return (
    <VStack gap={6}>
      {dialog}
      <Breadcrumbs>
        <BreadcrumbItem href="/routing">Routing</BreadcrumbItem>
        <BreadcrumbItem>{provider.name}</BreadcrumbItem>
      </Breadcrumbs>
      <HStack gap={3} vAlign="center" wrap="wrap">
        <VStack gap={1}>
          <Heading level={1}>{provider.name}</Heading>
          <HStack gap={2} vAlign="center">
            <Badge variant="blue" label={provider.provider_type} />
            {provider.api_dialect === "responses" ? (
              <Badge variant="purple" label="responses" />
            ) : null}
            <Badge
              variant={provider.is_active ? "success" : "neutral"}
              label={provider.is_active ? "Active" : "Paused"}
            />
          </HStack>
        </VStack>
        <HStack gap={2}>
          <Button label="Edit" variant="secondary" onClick={() => setIsEditing(true)} />
          <Button
            label="Delete"
            variant="destructive"
            onClick={() =>
              ask({
                title: `Delete ${provider.name}?`,
                description:
                  "Its routing rules and provider keys are removed with it. This cannot be undone.",
                actionLabel: "Delete provider",
                onConfirm: deleteProviderNow,
              })
            }
          />
        </HStack>
      </HStack>
      <ErrorBanner message={actionError} />
      <Section>
        <VStack gap={3}>
          <Text weight="semibold">Endpoint</Text>
          <HStack gap={6} wrap="wrap">
          <Field label="Base URL">
            <Text size="sm">{provider.base_url}</Text>
          </Field>
          <Field label="API key">
            <Text size="sm">{provider.api_key_masked || "—"}</Text>
          </Field>
          <Field label="Timeout">
            <Text size="sm" hasTabularNumbers>
              {provider.timeout_ms} ms
            </Text>
          </Field>
          </HStack>
        </VStack>
      </Section>
      <Section>
        <VStack gap={3}>
          <Text weight="semibold">Provider keys</Text>
          <ProviderKeysSection provider={provider} />
        </VStack>
      </Section>
      <Section>
        <VStack gap={3}>
          <HStack gap={2} vAlign="center" wrap="wrap">
            <Text weight="semibold">Models</Text>
            <Badge variant="neutral" label={`${provider.models.length}`} />
            <Button
              label="Sync from provider"
              variant="secondary"
              size="sm"
              onClick={syncModels}
              isLoading={isSyncing}
            />
          </HStack>
          <Text type="supporting" size="sm">
            Syncing replaces this list with what the provider reports, so retired
            models disappear.
          </Text>
          {provider.models.length === 0 ? (
            <EmptyState title="No models yet. Sync from the provider or add one below." />
          ) : (
            <VStack gap={2}>
              {provider.models.length > 8 ? (
                <HStack gap={2} vAlign="end">
                  <StackItem size="fill">
                    <TextInput
                      label="Filter models"
                      value={modelFilter}
                      onChange={setModelFilter}
                      placeholder={`Search ${provider.models.length} models`}
                      hasClear
                    />
                  </StackItem>
                  {needle ? (
                    <Text type="supporting" size="sm" hasTabularNumbers>
                      {visibleModels.length} of {provider.models.length}
                    </Text>
                  ) : null}
                </HStack>
              ) : null}
              {visibleModels.length === 0 ? (
                <Text type="supporting" size="sm">
                  No model matches “{modelFilter}”.
                </Text>
              ) : (
                <VStack
                  width="100%"
                  height={MODEL_LIST_MAX_HEIGHT}
                  isScrollable
                >
                  <Grid columns={{ minWidth: 260, max: 3 }} gap={2}>
                    {visibleModels.map((model) => (
                      <Card key={model} padding={1} variant="muted">
                        <Item
                          label={model}
                          labelLines={1}
                          density="compact"
                          endContent={
                            <Button
                              label={`Remove ${model}`}
                              variant="ghost"
                              size="sm"
                              isIconOnly
                              icon={<Icon icon={TrashIcon} size="sm" />}
                              onClick={() => void removeModel(model)}
                            />
                          }
                        />
                      </Card>
                    ))}
                  </Grid>
                </VStack>
              )}
            </VStack>
          )}
          <HStack gap={2} vAlign="end">
            <TextInput
              label="Add a model"
              value={newModel}
              onChange={setNewModel}
              placeholder="gpt-4o"
            />
            <Button
              label="Add"
              variant="primary"
              size="sm"
              onClick={() => void addModel()}
              isDisabled={newModel.trim() === ""}
            />
          </HStack>
        </VStack>
      </Section>
      <ProviderDialog
        isOpen={isEditing}
        onOpenChange={setIsEditing}
        provider={provider}
        onSaved={() => {
          setIsEditing(false);
          reload();
        }}
      />
    </VStack>
  );
}
