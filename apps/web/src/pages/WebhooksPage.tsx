import { useState } from "react";

import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Section } from "@astryxdesign/core/Section";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { HStack } from "@astryxdesign/core/HStack";
import { Heading } from "@astryxdesign/core/Heading";
import { Icon } from "@astryxdesign/core/Icon";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import type { TableColumn } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";
import { PaperAirplaneIcon, PencilSquareIcon, PlusIcon, TrashIcon } from "@heroicons/react/24/outline";

import { api } from "@/api/client";
import type { WebhookDto } from "@/api/types";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { useToast } from "@astryxdesign/core/Toast";
import { usePageAction } from "@/components/common/use-page-action";
import { usePageDetail } from "@/components/common/use-page-detail";
import { WebhookDialog } from "@/components/webhooks/WebhookDialog";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { queryKeys } from "@/api/queryKeys";

type WebhookRow = WebhookDto & Record<string, unknown>;

const webhookColumns: TableColumn<WebhookRow>[] = [
  {
    key: "name",
    header: "Webhook",
    width: proportional(1, { minWidth: 140 }),
    renderCell: (webhook) => (
      <VStack gap={0.5}>
        <Text weight="semibold">{webhook.name}</Text>
        <Text type="supporting" size="2xs">
          {webhook.url}
        </Text>
      </VStack>
    ),
  },
  {
    key: "events",
    header: "Events",
    width: proportional(1.2, { minWidth: 160 }),
    renderCell: (webhook) => (
      <HStack gap={1} wrap="wrap">
        {webhook.events.slice(0, 3).map((event) => (
          <Badge key={event} variant="neutral" label={event} />
        ))}
        {webhook.events.length > 3 ? (
          <Badge variant="neutral" label={`+${webhook.events.length - 3}`} />
        ) : null}
      </HStack>
    ),
  },
  {
    key: "last_status",
    header: "Last delivery",
    width: proportional(0.9, { minWidth: 150 }),
    renderCell: (webhook) => {
      if (webhook.last_status == null && !webhook.last_error) {
        return (
          <Text type="supporting" size="sm">
            Never sent
          </Text>
        );
      }
      return (
        <VStack gap={0.5}>
          {webhook.last_status != null ? (
            <Badge
              variant={webhook.last_status < 400 ? "success" : "error"}
              label={`HTTP ${webhook.last_status}`}
            />
          ) : null}
          {webhook.last_error ? (
            <Text type="supporting" size="2xs">
              {webhook.last_error}
            </Text>
          ) : null}
        </VStack>
      );
    },
  },
  {
    key: "is_active",
    header: "Status",
    width: pixel(90),
    renderCell: (webhook) => (
      <Badge
        variant={webhook.is_active ? "success" : "neutral"}
        label={webhook.is_active ? "Active" : "Paused"}
      />
    ),
  },
];

export function WebhooksPage() {
  const showToast = useToast();
  const { ask, dialog: confirmDialog } = useConfirmDialog();
  const queryClient = useQueryClient();
  const webhooksState = useQuery({
    queryKey: queryKeys.webhooks,
    queryFn: api.listWebhooks,
  });
  const [dialog, setDialog] = useState<{
    isOpen: boolean;
    webhook: WebhookDto | null;
  }>({
    isOpen: false,
    webhook: null,
  });
  const [actionError, setActionError] = useState<string | null>(null);
  const [testNotice, setTestNotice] = useState<string | null>(null);
  const [testingId, setTestingId] = useState<string | null>(null);

  const webhooks = webhooksState.data ?? [];

  async function removeWebhookNow(webhook: WebhookDto) {
    setActionError(null);
    try {
      await api.deleteWebhook(webhook.id);
      void queryClient.invalidateQueries({ queryKey: queryKeys.webhooks });
      showToast({ body: "Webhook deleted", uniqueID: `delete-webhook-${webhook.id}` });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  function removeWebhook(webhook: WebhookDto) {
    ask({
      title: `Delete ${webhook.name}?`,
      description:
        "Gateway events stop being delivered to this endpoint. This cannot be undone.",
      actionLabel: "Delete webhook",
      onConfirm: () => removeWebhookNow(webhook),
    });
  }

  async function sendTest(webhook: WebhookDto) {
    setActionError(null);
    setTestNotice(null);
    setTestingId(webhook.id);
    try {
      const result = await api.testWebhook(webhook.id);
      setTestNotice(
        result.delivered
          ? `Test to ${webhook.name} delivered: HTTP ${result.status ?? "—"}.`
          : `Test to ${webhook.name} failed: ${result.error ?? `HTTP ${result.status ?? "—"}`}.`,
      );
      void webhooksState.refetch();
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setTestingId(null);
    }
  }

  usePageDetail(
    () => `${webhooks.length} ${webhooks.length === 1 ? 'destination' : 'destinations'}`,
    [webhooksState.data],
  )

  usePageAction(
    () => (
      <Button
        label="New webhook"
        variant="primary"
        icon={<Icon icon={PlusIcon} />}
        onClick={() => setDialog({ isOpen: true, webhook: null })}
      />
    ),
    [],
  )

  return (
    <VStack gap={4}>
      {confirmDialog}
      <ErrorBanner message={webhooksState.error?.message ?? actionError} />
      {testNotice ? (
        <Banner
          status="info"
          title="Send test"
          description={testNotice}
          isDismissable
          onDismiss={() => setTestNotice(null)}
        />
      ) : null}
      <Section padding={0}>
        <VStack gap={3}>
          <Heading level={3}>Destinations</Heading>
          <Table
            data={webhooks as WebhookRow[]}
            idKey="id"
            density="spacious"
            dividers="rows"
            hasHover
            textOverflow="truncate"
            columns={[
              ...webhookColumns,
              {
                key: "actions",
                header: "",
                width: pixel(112),
                renderCell: (webhook) => (
                  <HStack gap={2} justify="end">
                    <Button
                      label={`Send test to ${webhook.name}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={PaperAirplaneIcon} />}
                      onClick={() => sendTest(webhook)}
                      isLoading={testingId === webhook.id}
                    />
                    <Button
                      label={`Edit ${webhook.name}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={PencilSquareIcon} />}
                      onClick={() => setDialog({ isOpen: true, webhook })}
                    />
                    <Button
                      label={`Delete ${webhook.name}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={TrashIcon} />}
                      onClick={() => removeWebhook(webhook)}
                    />
                  </HStack>
                ),
              },
            ]}
            emptyState={
              <EmptyState
                title="No webhook destinations yet"
                description="Add an endpoint to receive alerts and spend warnings."
                isCompact
              />
            }
          />
        </VStack>
      </Section>
      {dialog.isOpen ? (
        <WebhookDialog
          key={dialog.webhook?.id ?? "new-webhook"}
          isOpen
          onOpenChange={(isOpen) => {
            if (!isOpen) setDialog({ isOpen: false, webhook: null });
          }}
          webhook={dialog.webhook}
          onSaved={() => {
            setDialog({ isOpen: false, webhook: null });
            void webhooksState.refetch();
          }}
        />
      ) : null}
    </VStack>
  );
}
