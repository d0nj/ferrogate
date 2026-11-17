import { useState } from "react";

import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { Section } from "@astryxdesign/core/Section";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Code } from "@astryxdesign/core/Code";
import { HStack } from "@astryxdesign/core/HStack";
import { Heading } from "@astryxdesign/core/Heading";
import { Icon } from "@astryxdesign/core/Icon";
import { IconButton } from "@astryxdesign/core/IconButton";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import type { TableColumn } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";
import { PencilSquareIcon, PlusIcon, TrashIcon } from "@heroicons/react/24/outline";

import { api } from "@/api/client";
import type { GuardrailDto } from "@/api/types";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { useToast } from "@astryxdesign/core/Toast";
import { usePageAction } from "@/components/common/use-page-action";
import { usePageDetail } from "@/components/common/use-page-detail";
import { GuardrailDialog } from "@/components/guardrails/GuardrailDialog";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { queryKeys } from "@/api/queryKeys";

type GuardrailRow = GuardrailDto & Record<string, unknown>;

const guardrailColumns: TableColumn<GuardrailRow>[] = [
  {
    key: "name",
    header: "Guardrail",
    width: proportional(1.2, { minWidth: 160 }),
    renderCell: (rule) => <Text weight="semibold">{rule.name}</Text>,
  },
  {
    key: "phase",
    header: "Phase",
    width: pixel(100),
    renderCell: (rule) => (
      <Badge
        variant={rule.phase === "input" ? "blue" : "cyan"}
        label={rule.phase}
      />
    ),
  },
  {
    key: "rule_type",
    header: "Type",
    width: pixel(150),
    renderCell: (rule) => <Badge variant="neutral" label={rule.rule_type} />,
  },
  {
    key: "pattern",
    header: "Pattern",
    width: proportional(1.4, { minWidth: 180 }),
    renderCell: (rule) =>
      rule.pattern ? (
        <Code>{rule.pattern}</Code>
      ) : (
        <Text type="supporting" size="sm">
          Built-in detector
        </Text>
      ),
  },
  {
    key: "action",
    header: "Action",
    width: pixel(100),
    renderCell: (rule) => (
      <Badge
        variant={
          rule.action === "block"
            ? "error"
            : rule.action === "redact"
              ? "warning"
              : "neutral"
        }
        label={rule.action}
      />
    ),
  },
  {
    key: "priority",
    header: "Priority",
    align: "end",
    width: pixel(90),
    renderCell: (rule) => (
      <Text size="sm" hasTabularNumbers>
        {rule.priority}
      </Text>
    ),
  },
  {
    key: "is_active",
    header: "Status",
    width: pixel(90),
    renderCell: (rule) => (
      <Badge
        variant={rule.is_active ? "success" : "neutral"}
        label={rule.is_active ? "Active" : "Paused"}
      />
    ),
  },
];

export function GuardrailsPage() {
  const showToast = useToast();
  const { ask, dialog: confirmDialog } = useConfirmDialog();
  const queryClient = useQueryClient();
  const guardrailsState = useQuery({
    queryKey: queryKeys.guardrails,
    queryFn: api.listGuardrails,
  });
  const [dialog, setDialog] = useState<{
    isOpen: boolean;
    guardrail: GuardrailDto | null;
  }>({
    isOpen: false,
    guardrail: null,
  });
  const [actionError, setActionError] = useState<string | null>(null);

  const guardrails = guardrailsState.data ?? [];

  async function removeGuardrailNow(guardrail: GuardrailDto) {
    setActionError(null);
    try {
      await api.deleteGuardrail(guardrail.id);
      void queryClient.invalidateQueries({ queryKey: queryKeys.guardrails });
      showToast({ body: "Guardrail deleted", uniqueID: `delete-guardrail-${guardrail.id}` });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  function removeGuardrail(guardrail: GuardrailDto) {
    ask({
      title: `Delete ${guardrail.name}?`,
      description:
        "Requests are no longer checked against this rule. This cannot be undone.",
      actionLabel: "Delete rule",
      onConfirm: () => removeGuardrailNow(guardrail),
    });
  }

  usePageDetail(
    () => `${guardrails.length} ${guardrails.length === 1 ? 'rule' : 'rules'}`,
    [guardrailsState.data],
  )

  usePageAction(
    () => (
      <Button
        label="New guardrail"
        variant="primary"
        icon={<Icon icon={PlusIcon} />}
        onClick={() => setDialog({ isOpen: true, guardrail: null })}
      />
    ),
    [],
  )

  return (
    <VStack gap={4}>
      {confirmDialog}
      <ErrorBanner message={guardrailsState.error?.message ?? actionError} />
      <Section padding={0}>
        <VStack gap={3}>
          <Heading level={3}>Rules</Heading>
          <Table
            data={guardrails as GuardrailRow[]}
            idKey="id"
            density="spacious"
            dividers="grid"
            hasHover
            textOverflow="truncate"
            columns={[
              ...guardrailColumns,
              {
                key: "actions",
                header: "",
                width: pixel(96),
                renderCell: (rule) => (
                  <HStack gap={2} justify="end">
                    <IconButton
                      label={`Edit ${rule.name}`}
                      tooltip={`Edit ${rule.name}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={PencilSquareIcon} />}
                      onClick={() =>
                        setDialog({ isOpen: true, guardrail: rule })
                      }
                    />
                    <IconButton
                      label={`Delete ${rule.name}`}
                      tooltip={`Delete ${rule.name}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={TrashIcon} />}
                      onClick={() => removeGuardrail(rule)}
                    />
                  </HStack>
                ),
              },
            ]}
            emptyState={
              <EmptyState
                title="No guardrail rules yet"
                description="Add a rule to block, redact or flag matched traffic."
                isCompact
              />
            }
          />
        </VStack>
      </Section>
      {dialog.isOpen ? (
        <GuardrailDialog
          key={dialog.guardrail?.id ?? "new-guardrail"}
          isOpen
          onOpenChange={(isOpen) => {
            if (!isOpen) setDialog({ isOpen: false, guardrail: null });
          }}
          guardrail={dialog.guardrail}
          onSaved={() => {
            setDialog({ isOpen: false, guardrail: null });
            void guardrailsState.refetch();
          }}
        />
      ) : null}
    </VStack>
  );
}
