import { useState } from "react";

import { Button } from "@astryxdesign/core/Button";
import { Icon } from "@astryxdesign/core/Icon";
import { Section } from "@astryxdesign/core/Section";
import { VStack } from "@astryxdesign/core/VStack";
import { PlusIcon } from "@heroicons/react/24/outline";

import { api } from "@/api/client";
import type { VirtualKeyDto } from "@/api/types";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { useToast } from "@astryxdesign/core/Toast";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { usePageAction } from "@/components/common/use-page-action";
import { usePageDetail } from "@/components/common/use-page-detail";
import { CreateKeyDialog } from "@/components/keys/CreateKeyDialog";
import { EditKeyDialog } from "@/components/keys/EditKeyDialog";
import { KeysTable } from "@/components/keys/KeysTable";
import { TeamsStrip } from "@/components/keys/TeamsStrip";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { queryKeys } from "@/api/queryKeys";

function collectModels(keys: VirtualKeyDto[]): string[] {
  const models = new Set<string>();
  for (const key of keys) {
    for (const model of key.allowed_models) models.add(model);
  }
  return [...models].sort();
}

function collectTags(keys: VirtualKeyDto[]): string[] {
  const tags = new Set<string>();
  for (const key of keys) {
    for (const tag of key.tags) tags.add(tag);
  }
  return [...tags].sort();
}

function collectMcp(keys: VirtualKeyDto[]): string[] {
  const servers = new Set<string>();
  for (const key of keys) {
    for (const server of key.allowed_mcp) servers.add(server);
  }
  return [...servers].sort();
}

export function KeysPage() {
  const showToast = useToast();
  const queryClient = useQueryClient();
  const { ask, dialog } = useConfirmDialog();
  const keysState = useQuery({
    queryKey: queryKeys.keys,
    queryFn: api.listKeys,
  });
  const [isCreateOpen, setCreateOpen] = useState(false);
  const [editingKey, setEditingKey] = useState<VirtualKeyDto | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const data = keysState.data ?? null;
  const keys = keysState.data ?? [];

  function invalidateKeys() {
    void queryClient.invalidateQueries({ queryKey: queryKeys.keys });
  }

  usePageDetail(
    () => {
      const active = keys.filter((key) => key.is_active).length;
      return `${keys.length} ${keys.length === 1 ? 'key' : 'keys'} · ${active} active`;
    },
    [data],
  )

  usePageAction(
    () => (
      <Button
        label="Create key"
        variant="primary"
        icon={<Icon icon={PlusIcon} />}
        onClick={() => setCreateOpen(true)}
      />
    ),
    [],
  )

  async function toggleActive(key: VirtualKeyDto, isActive: boolean) {
    setActionError(null);
    try {
      await api.updateKey(key.id, { is_active: isActive });
      invalidateKeys();
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  async function revokeNow(key: VirtualKeyDto) {
    setActionError(null);
    try {
      await api.deleteKey(key.id);
      invalidateKeys();
      showToast({ body: "Key revoked", uniqueID: `revoke-key-${key.id}` });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  function revoke(key: VirtualKeyDto) {
    ask({
      title: `Revoke ${key.name}?`,
      description:
        "Requests using this key stop working immediately. This cannot be undone.",
      actionLabel: "Revoke key",
      onConfirm: () => revokeNow(key),
    });
  }

  return (
    <VStack gap={4}>
      {dialog}
      <ErrorBanner message={keysState.error?.message ?? actionError} />
      <TeamsStrip />
      <Section padding={0}>
        <KeysTable
          keys={keys}
          onToggleActive={toggleActive}
          onEdit={setEditingKey}
          onRevoke={revoke}
        />
      </Section>
      <CreateKeyDialog
        isOpen={isCreateOpen}
        onOpenChange={setCreateOpen}
        models={collectModels(keys)}
        allTags={collectTags(keys)}
        allMcp={collectMcp(keys)}
        onCreated={invalidateKeys}
      />
      {editingKey ? (
        <EditKeyDialog
          key={editingKey.id}
          isOpen
          onOpenChange={(isOpen) => {
            if (!isOpen) setEditingKey(null);
          }}
          virtualKey={editingKey}
          models={collectModels(keys)}
          allTags={collectTags(keys)}
          allMcp={collectMcp(keys)}
          onSaved={() => {
            setEditingKey(null);
            invalidateKeys();
          }}
        />
      ) : null}
    </VStack>
  );
}
