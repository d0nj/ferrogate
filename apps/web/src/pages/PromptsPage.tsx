import { useState } from "react";

import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { Section } from "@astryxdesign/core/Section";
import { Code } from "@astryxdesign/core/Code";
import { CodeBlock } from "@astryxdesign/core/CodeBlock";
import { Divider } from "@astryxdesign/core/Divider";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Drawer, DrawerHeader } from "@astryxdesign/core/Drawer";
import { HStack } from "@astryxdesign/core/HStack";
import { Heading } from "@astryxdesign/core/Heading";
import { Icon } from "@astryxdesign/core/Icon";
import { IconButton } from "@astryxdesign/core/IconButton";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import type { TableColumn } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { TextArea } from "@astryxdesign/core/TextArea";
import { VStack } from "@astryxdesign/core/VStack";
import { EyeIcon, PencilSquareIcon, PlusIcon, TrashIcon } from "@heroicons/react/24/outline";

import { api } from "@/api/client";
import type {
  PromptRenderResponse,
  PromptSummaryDto,
  PromptVersionDto,
} from "@/api/types";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { useToast } from "@astryxdesign/core/Toast";
import { usePageAction } from "@/components/common/use-page-action";
import { usePageDetail } from "@/components/common/use-page-detail";
import { formatPromptVariables } from "@/components/prompts/constants";
import { PromptDialog } from "@/components/prompts/PromptDialog";
import { queryKeys } from "@/api/queryKeys";
import { useQuery, useQueryClient } from "@tanstack/react-query";

type PromptDialogMode = "create" | "edit" | "version";

type PromptRow = PromptSummaryDto & Record<string, unknown>;

const promptColumns: TableColumn<PromptRow>[] = [
  {
    key: "slug",
    header: "Prompt",
    width: proportional(1, { minWidth: 160 }),
    renderCell: (prompt) => (
      <VStack gap={0.5}>
        <Text weight="semibold">{prompt.slug}</Text>
        <Text type="supporting" size="2xs">
          {prompt.name} · v{prompt.latest_version}
        </Text>
      </VStack>
    ),
  },
  {
    key: "model",
    header: "Model",
    width: proportional(0.9, { minWidth: 140 }),
    renderCell: (prompt) =>
      prompt.model ? (
        <Code>{prompt.model}</Code>
      ) : (
        <Text type="supporting" size="sm">
          Default
        </Text>
      ),
  },
  {
    key: "variables",
    header: "Variables",
    width: proportional(0.9, { minWidth: 140 }),
    renderCell: (prompt) =>
      prompt.variables.length > 0 ? (
        <HStack gap={1} wrap="wrap">
          {prompt.variables.slice(0, 3).map((variable) => (
            <Badge
              key={variable.name}
              variant="neutral"
              label={variable.name}
            />
          ))}
          {prompt.variables.length > 3 ? (
            <Badge
              variant="neutral"
              label={`+${prompt.variables.length - 3}`}
            />
          ) : null}
        </HStack>
      ) : (
        <Text type="supporting" size="sm">
          None
        </Text>
      ),
  },
  {
    key: "is_active",
    header: "Status",
    width: pixel(100),
    renderCell: (prompt) => (
      <Badge
        variant={prompt.is_active ? "success" : "neutral"}
        label={prompt.is_active ? "Active" : "Inactive"}
      />
    ),
  },
];

function VersionPreview({
  slug,
  version,
}: {
  slug: string;
  version: PromptVersionDto;
}) {
  const [values, setValues] = useState(() =>
    formatPromptVariables(version.variables),
  );
  const [result, setResult] = useState<PromptRenderResponse | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [isRendering, setIsRendering] = useState(false);

  async function render() {
    setPreviewError(null);
    setIsRendering(true);
    try {
      const variables: Record<string, string> = {};
      for (const line of values.split("\n")) {
        const separator = line.indexOf("=");
        if (separator === -1) continue;
        const name = line.slice(0, separator).trim();
        if (name) variables[name] = line.slice(separator + 1).trim();
      }
      setResult(await api.renderPrompt(slug, variables, version.version));
    } catch (cause) {
      setResult(null);
      setPreviewError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsRendering(false);
    }
  }

  return (
    <VStack gap={2}>
      <Text weight="semibold" size="sm">
        Live preview · v{version.version}
      </Text>
      <TextArea
        label="Preview values"
        value={values}
        onChange={setValues}
        placeholder={"topic=ferrogate"}
        description="One per line: name=value."
      />
      <HStack gap={2}>
        <Button
          label={`Render version ${version.version}`}
          variant="secondary"
          size="sm"
          onClick={render}
          isLoading={isRendering}
        />
      </HStack>
      <ErrorBanner message={previewError} />
      {result ? (
        <VStack gap={2}>
          <Text type="supporting" size="2xs">
            System
          </Text>
          <CodeBlock
            code={result.system || "(empty)"}
            hasCopyButton
            isWrapped
            maxHeight={200}
          />
          <Text type="supporting" size="2xs">
            User
          </Text>
          <CodeBlock
            code={result.user || "(empty)"}
            hasCopyButton
            isWrapped
            maxHeight={200}
          />
          {result.missing_variables.length > 0 ? (
            <HStack gap={1} wrap="wrap">
              {result.missing_variables.map((name) => (
                <Badge
                  key={name}
                  variant="warning"
                  label={`missing: ${name}`}
                />
              ))}
            </HStack>
          ) : null}
        </VStack>
      ) : null}
    </VStack>
  );
}

function PromptDrawer({
  slug,
  onOpenChange,
  onChanged,
}: {
  slug: string | null;
  onOpenChange: (isOpen: boolean) => void;
  onChanged: () => void;
}) {
  const versionsState = useQuery({
    queryKey: queryKeys.prompt(slug ?? ""),
    queryFn: () =>
      slug
        ? api.listPromptVersions(slug)
        : Promise.resolve({ slug: "", versions: [] }),
    enabled: slug !== null,
  });
  const { data } = versionsState;
  const error = versionsState.error?.message ?? null;
  const [actionError, setActionError] = useState<string | null>(null);
  const [editing, setEditing] = useState<PromptVersionDto | null>(null);
  const [activating, setActivating] = useState<number | null>(null);

  const versions = slug ? (data?.versions ?? []) : [];

  const queryClient = useQueryClient();

  async function activate(version: PromptVersionDto) {
    setActionError(null);
    setActivating(version.version);
    try {
      await api.activatePrompt(version.slug, version.version);
      void queryClient.invalidateQueries({
        queryKey: queryKeys.prompt(version.slug),
      });
      onChanged();
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setActivating(null);
    }
  }

  return (
    <Drawer
      isOpen={slug !== null}
      onOpenChange={onOpenChange}
      side="end"
      width={560}
      label="Prompt template versions"
    >
      {slug ? (
        <VStack gap={4}>
          <DrawerHeader
            title={slug}
            subtitle={`${versions.length} versions`}
            onOpenChange={onOpenChange}
          />
          <ErrorBanner message={error ?? actionError} />
          {versions.map((version) => (
            <VStack key={version.id} gap={2}>
              <HStack hAlign="between" vAlign="center" gap={2} wrap="wrap">
                <HStack gap={2} vAlign="center">
                  <Text weight="semibold" size="sm">
                    Version {version.version}
                  </Text>
                  {version.is_active ? (
                    <Badge variant="success" label="Active" />
                  ) : null}
                </HStack>
                <HStack gap={2}>
                  {!version.is_active ? (
                    <Button
                      label={`Make version ${version.version} active`}
                      variant="secondary"
                      size="sm"
                      onClick={() => activate(version)}
                      isLoading={activating === version.version}
                    />
                  ) : null}
                  <IconButton
                    label={`Edit version ${version.version}`}
                    tooltip={`Edit version ${version.version}`}
                    variant="ghost"
                    size="sm"
                    icon={<Icon icon={PencilSquareIcon} />}
                    onClick={() => setEditing(version)}
                  />
                </HStack>
              </HStack>
              <Text type="supporting" size="2xs">
                System
              </Text>
              <CodeBlock
                code={version.system_prompt || "(empty)"}
                isCollapsible
                isWrapped
                maxHeight={160}
              />
              <Text type="supporting" size="2xs">
                User template
              </Text>
              <CodeBlock
                code={version.user_template || "(empty)"}
                isCollapsible
                isWrapped
                maxHeight={160}
              />
              <VersionPreview slug={slug} version={version} />
              <Divider />
            </VStack>
          ))}
        </VStack>
      ) : null}
      {editing ? (
        <PromptDialog
          key={editing.id}
          isOpen
          mode="edit"
          onOpenChange={(isOpen) => {
            if (!isOpen) setEditing(null);
          }}
          prompt={editing}
          onSaved={() => {
            setEditing(null);
            void queryClient.invalidateQueries({
              queryKey: queryKeys.prompt(editing.slug),
            });
            onChanged();
          }}
        />
      ) : null}
    </Drawer>
  );
}

export function PromptsPage() {
  const showToast = useToast();
  const { ask, dialog: confirmDialog } = useConfirmDialog();
  const queryClient = useQueryClient();
  const promptsState = useQuery({
    queryKey: queryKeys.prompts,
    queryFn: api.listPrompts,
  });
  const data = promptsState.data;
  const error = promptsState.error?.message ?? null;
  const [dialog, setDialog] = useState<{
    isOpen: boolean;
    mode: PromptDialogMode;
    prompt: PromptVersionDto | null;
  }>({
    isOpen: false,
    mode: "create",
    prompt: null,
  });
  const [selectedSlug, setSelectedSlug] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const prompts = data ?? [];

  async function removePromptNow(prompt: PromptSummaryDto) {
    setActionError(null);
    try {
      await api.deletePrompt(prompt.slug);
      void queryClient.invalidateQueries({ queryKey: queryKeys.prompts });
      showToast({ body: "Prompt deleted", uniqueID: `delete-prompt-${prompt.slug}` });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  function removePrompt(prompt: PromptSummaryDto) {
    ask({
      title: `Delete ${prompt.slug}?`,
      description: "Every version of this prompt is removed. This cannot be undone.",
      actionLabel: "Delete prompt",
      onConfirm: () => removePromptNow(prompt),
    });
  }

  async function createNewVersion(prompt: PromptSummaryDto) {
    setActionError(null);
    try {
      const details = await api.listPromptVersions(prompt.slug);
      const latest = details.versions[0];
      if (!latest) {
        setActionError(`No versions found for ${prompt.slug}.`);
        return;
      }
      setDialog({ isOpen: true, mode: "version", prompt: latest });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  usePageDetail(
    () => `${prompts.length} ${prompts.length === 1 ? 'template' : 'templates'}`,
    [data],
  )

  usePageAction(
    () => (
      <Button
        label="New prompt"
        variant="primary"
        icon={<Icon icon={PlusIcon} />}
        onClick={() => setDialog({ isOpen: true, mode: "create", prompt: null })}
      />
    ),
    [],
  )

  return (
    <VStack gap={4}>
      {confirmDialog}
      <ErrorBanner message={error ?? actionError} />
      <Section padding={0}>
        <VStack gap={3}>
          <Heading level={3}>Templates</Heading>
          <Table
            data={prompts as PromptRow[]}
            idKey="slug"
            density="spacious"
            dividers="rows"
            hasHover
            textOverflow="truncate"
            columns={[
              ...promptColumns,
              {
                key: "actions",
                header: "",
                width: pixel(120),
                renderCell: (prompt) => (
                  <HStack gap={2} justify="end">
                    <IconButton
                      label={`Inspect ${prompt.slug}`}
                      tooltip={`Inspect ${prompt.slug}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={EyeIcon} />}
                      onClick={() => setSelectedSlug(prompt.slug)}
                    />
                    <IconButton
                      label={`New version of ${prompt.slug}`}
                      tooltip={`New version of ${prompt.slug}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={PlusIcon} />}
                      onClick={() => createNewVersion(prompt)}
                    />
                    <IconButton
                      label={`Delete ${prompt.slug}`}
                      tooltip={`Delete ${prompt.slug}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={TrashIcon} />}
                      onClick={() => removePrompt(prompt)}
                    />
                  </HStack>
                ),
              },
            ]}
            emptyState={
              <EmptyState
                title="No prompt templates yet"
                description="Create a template to reuse system prompts and variables."
                isCompact
              />
            }
          />
        </VStack>
      </Section>
      <PromptDrawer
        slug={selectedSlug}
        onOpenChange={(isOpen) => {
          if (!isOpen) setSelectedSlug(null);
        }}
        onChanged={() => void promptsState.refetch()}
      />
      {dialog.isOpen ? (
        <PromptDialog
          key={dialog.prompt?.id ?? "new-prompt"}
          isOpen
          mode={dialog.mode}
          onOpenChange={(isOpen) => {
            if (!isOpen)
              setDialog({ isOpen: false, mode: "create", prompt: null });
          }}
          prompt={dialog.prompt}
          onSaved={() => {
            setDialog({ isOpen: false, mode: "create", prompt: null });
            void queryClient.invalidateQueries({ queryKey: queryKeys.prompts });
          }}
        />
      ) : null}
    </VStack>
  );
}
