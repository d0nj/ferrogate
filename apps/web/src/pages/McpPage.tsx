import { useState } from "react";

import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { Section } from "@astryxdesign/core/Section";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { HStack } from "@astryxdesign/core/HStack";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import type { TableColumn } from "@astryxdesign/core/Table";
import { Heading } from "@astryxdesign/core/Heading";
import { Icon } from "@astryxdesign/core/Icon";
import { IconButton } from "@astryxdesign/core/IconButton";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";
import { ArrowPathIcon, PencilSquareIcon, PlusIcon, TrashIcon } from "@heroicons/react/24/outline";

import { api } from "@/api/client";
import type { McpServerDto, McpToolEntryDto } from "@/api/types";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { useToast } from "@astryxdesign/core/Toast";
import { usePageAction } from "@/components/common/use-page-action";
import { usePageDetail } from "@/components/common/use-page-detail";
import { McpServerDialog } from "@/components/mcp/McpServerDialog";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { queryKeys } from "@/api/queryKeys";

type McpServerRow = McpServerDto & Record<string, unknown>;
type McpToolRow = McpToolEntryDto & Record<string, unknown>;

function statusVariant(server: McpServerDto): "success" | "error" | "neutral" {
  if (!server.is_active) return "neutral";
  if (server.last_status === "ready") return "success";
  if (server.last_status === "unreachable") return "error";
  return "neutral";
}

function statusLabel(server: McpServerDto): string {
  if (!server.is_active) return "Paused";
  return server.last_status ?? "never_synced";
}

const serverColumns: TableColumn<McpServerRow>[] = [
  {
    key: "name",
    header: "Server",
    width: proportional(1, { minWidth: 140 }),
    renderCell: (server) => (
      <VStack gap={0.5}>
        <Text weight="semibold">{server.name}</Text>
        <Text type="supporting" size="2xs">
          {server.url}
        </Text>
      </VStack>
    ),
  },
  {
    key: "transport",
    header: "Transport",
    width: pixel(100),
    renderCell: (server) => (
      <Badge variant="neutral" label={server.transport} />
    ),
  },
  {
    key: "last_status",
    header: "Status",
    width: pixel(130),
    renderCell: (server) => (
      <Badge variant={statusVariant(server)} label={statusLabel(server)} />
    ),
  },
  {
    key: "tool_count",
    header: "Tools",
    align: "end",
    width: pixel(80),
    renderCell: (server) => (
      <Text size="sm" hasTabularNumbers>
        {server.tool_count}
      </Text>
    ),
  },
];

const toolColumns: TableColumn<McpToolRow>[] = [
  {
    key: "name",
    header: "Tool",
    width: proportional(1, { minWidth: 160 }),
    renderCell: (tool) => (
      <VStack gap={0.5}>
        <Text weight="semibold">{tool.name}</Text>
        <Text type="supporting" size="2xs">
          {tool.server}
        </Text>
      </VStack>
    ),
  },
  {
    key: "description",
    header: "Description",
    width: proportional(1.4, { minWidth: 200 }),
    renderCell: (tool) =>
      tool.description ? (
        <Text size="sm">{tool.description}</Text>
      ) : (
        <Text type="supporting" size="sm">
          No description
        </Text>
      ),
  },
];

export function McpPage() {
  const showToast = useToast();
  const { ask, dialog: confirmDialog } = useConfirmDialog();
  const queryClient = useQueryClient();
  const serversState = useQuery({
    queryKey: queryKeys.mcpServers,
    queryFn: api.listMcpServers,
  });
  const toolsState = useQuery({
    queryKey: queryKeys.mcpTools,
    queryFn: api.listMcpTools,
  });
  const [dialog, setDialog] = useState<{
    isOpen: boolean;
    server: McpServerDto | null;
  }>({
    isOpen: false,
    server: null,
  });
  const [actionError, setActionError] = useState<string | null>(null);
  const [refreshingId, setRefreshingId] = useState<string | null>(null);

  const servers = serversState.data ?? [];
  const tools = toolsState.data ?? [];

  function reloadAll() {
    void queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers });
    void queryClient.invalidateQueries({ queryKey: queryKeys.mcpTools });
  }

  async function removeServerNow(server: McpServerDto) {
    setActionError(null);
    try {
      await api.deleteMcpServer(server.id);
      reloadAll();
      showToast({ body: "Server removed", uniqueID: `delete-mcp-${server.id}` });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  function removeServer(server: McpServerDto) {
    ask({
      title: `Remove ${server.name}?`,
      description:
        "Its tools disappear from the gateway immediately. This cannot be undone.",
      actionLabel: "Remove server",
      onConfirm: () => removeServerNow(server),
    });
  }

  async function refreshServer(server: McpServerDto) {
    setActionError(null);
    setRefreshingId(server.id);
    try {
      await api.refreshMcpServer(server.id);
      reloadAll();
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setRefreshingId(null);
    }
  }

  usePageDetail(
    () => `${servers.length} ${servers.length === 1 ? 'server' : 'servers'}`,
    [serversState.data],
  )

  usePageAction(
    () => (
      <Button
        label="New server"
        variant="primary"
        icon={<Icon icon={PlusIcon} />}
        onClick={() => setDialog({ isOpen: true, server: null })}
      />
    ),
    [],
  )

  return (
    <VStack gap={4}>
      {confirmDialog}
      <ErrorBanner
        message={serversState.error?.message ?? toolsState.error?.message ?? actionError}
      />
      <Section padding={0}>
        <VStack gap={3}>
          <Heading level={3}>Servers</Heading>
          <Table
            data={servers as McpServerRow[]}
            idKey="id"
            density="spacious"
            dividers="rows"
            hasHover
            textOverflow="truncate"
            columns={[
              ...serverColumns,
              {
                key: "actions",
                header: "",
                width: pixel(112),
                renderCell: (server) => (
                  <HStack gap={2} justify="end">
                    <IconButton
                      label={`Refresh ${server.name}`}
                      tooltip={`Refresh ${server.name}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={ArrowPathIcon} />}
                      onClick={() => refreshServer(server)}
                      isLoading={refreshingId === server.id}
                    />
                    <IconButton
                      label={`Edit ${server.name}`}
                      tooltip={`Edit ${server.name}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={PencilSquareIcon} />}
                      onClick={() => setDialog({ isOpen: true, server })}
                    />
                    <IconButton
                      label={`Delete ${server.name}`}
                      tooltip={`Delete ${server.name}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={TrashIcon} />}
                      onClick={() => removeServer(server)}
                    />
                  </HStack>
                ),
              },
            ]}
            emptyState={
              <EmptyState
                title="No MCP servers yet"
                description="Register a server to expose its tools through the gateway."
                isCompact
              />
            }
          />
        </VStack>
      </Section>
      <Section padding={0}>
        <VStack gap={3}>
          <Heading level={3}>Tools</Heading>
          <Table
            data={tools as McpToolRow[]}
            idKey="name"
            density="spacious"
            dividers="rows"
            textOverflow="truncate"
            columns={toolColumns}
            emptyState={
              <EmptyState
                title="No MCP tools discovered yet"
                description="Refresh a server to pull in its tool catalogue."
                isCompact
              />
            }
          />
        </VStack>
      </Section>
      {dialog.isOpen ? (
        <McpServerDialog
          key={dialog.server?.id ?? "new-mcp-server"}
          isOpen
          onOpenChange={(isOpen) => {
            if (!isOpen) setDialog({ isOpen: false, server: null });
          }}
          server={dialog.server}
          onSaved={() => {
            setDialog({ isOpen: false, server: null });
            reloadAll();
          }}
        />
      ) : null}
    </VStack>
  );
}
