import { useState } from "react";

import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import type { TableColumn } from "@astryxdesign/core/Table";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { HStack } from "@astryxdesign/core/HStack";
import { Heading } from "@astryxdesign/core/Heading";
import { Icon } from "@astryxdesign/core/Icon";
import { IconButton } from "@astryxdesign/core/IconButton";
import { Section } from "@astryxdesign/core/Section";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";
import { KeyIcon, PencilSquareIcon, PlusIcon, TrashIcon } from "@heroicons/react/24/outline";

import { api } from "@/api/client";
import type { Role } from "@/api/auth";
import { roleRank } from "@/api/auth";
import { useCurrentUser } from "@/api/current-user";
import type { UserDto } from "@/api/types";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { useToast } from "@astryxdesign/core/Toast";
import { usePageAction } from "@/components/common/use-page-action";
import { usePageDetail } from "@/components/common/use-page-detail";
import { ResetPasswordDialog, UserDialog } from "@/components/users/UserDialog";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { queryKeys } from "@/api/queryKeys";
import { formatTime } from "@/utils/format";

type UserRow = UserDto & Record<string, unknown>;

function roleBadgeVariant(role: Role): "warning" | "blue" | "neutral" {
  switch (roleRank(role)) {
    case 3:
      return "warning";
    case 2:
      return "blue";
    default:
      return "neutral";
  }
}

const userColumns: TableColumn<UserRow>[] = [
  {
    key: "user",
    header: "User",
    width: proportional(1.4, { minWidth: 180 }),
    renderCell: (user) => (
      <VStack gap={0.5}>
        <Text weight="semibold">{user.name}</Text>
        <Text type="supporting" size="2xs">
          {user.email}
        </Text>
      </VStack>
    ),
  },
  {
    key: "role",
    header: "Role",
    width: pixel(110),
    renderCell: (user) => (
      <Badge variant={roleBadgeVariant(user.role)} label={user.role} />
    ),
  },
  {
    key: "status",
    header: "Status",
    width: pixel(100),
    renderCell: (user) => (
      <Badge variant={user.is_active ? "success" : "neutral"} label={user.is_active ? "Active" : "Paused"} />
    ),
  },
  {
    key: "last_login",
    header: "Last login",
    width: proportional(1, { minWidth: 150 }),
    renderCell: (user) => (
      <Text type="supporting" size="sm">
        {user.last_login_at ? formatTime(user.last_login_at) : "Never"}
      </Text>
    ),
  },
];

export function UsersPage() {
  const showToast = useToast();
  const { ask, dialog } = useConfirmDialog();
  const currentUser = useCurrentUser();
  const queryClient = useQueryClient();
  const usersState = useQuery({
    queryKey: queryKeys.users,
    queryFn: api.listUsers,
  });
  const [editDialog, setEditDialog] = useState<{
    isOpen: boolean;
    user: UserDto | null;
  }>({ isOpen: false, user: null });
  const [resetDialog, setResetDialog] = useState<{
    isOpen: boolean;
    user: UserDto | null;
  }>({ isOpen: false, user: null });
  const [actionError, setActionError] = useState<string | null>(null);

  const users = usersState.data ?? [];

  async function removeUserNow(user: UserDto) {
    if (!user.id) {
      setActionError("This account cannot be deleted.");
      return;
    }
    setActionError(null);
    try {
      await api.deleteUser(user.id);
      void queryClient.invalidateQueries({ queryKey: queryKeys.users });
      showToast({ body: "User deleted", uniqueID: `delete-user-${user.id}` });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  function removeUser(user: UserDto) {
    ask({
      title: `Delete ${user.email}?`,
      description:
        "The account loses access to this console immediately. This cannot be undone.",
      actionLabel: "Delete user",
      onConfirm: () => removeUserNow(user),
    });
  }

  usePageDetail(
    () => `${users.length} ${users.length === 1 ? 'account' : 'accounts'}`,
    [usersState.data],
  )

  usePageAction(
    () => (
      <Button
        label="New user"
        variant="primary"
        icon={<Icon icon={PlusIcon} />}
        onClick={() => setEditDialog({ isOpen: true, user: null })}
      />
    ),
    [],
  )

  return (
    <VStack gap={4}>
      {dialog}
      <ErrorBanner message={usersState.error?.message ?? actionError} />
      <Section padding={0}>
        <VStack gap={3}>
          <Heading level={3}>Accounts</Heading>
          <Table
            data={users as UserRow[]}
            idKey={(user) => user.id ?? user.email}
            density="spacious"
            dividers="rows"
            hasHover
            textOverflow="truncate"
            columns={[
              ...userColumns,
              {
                key: "actions",
                header: "",
                width: pixel(140),
                renderCell: (user) => (
                  <HStack gap={2} justify="end">
                    <IconButton
                      label={`Edit ${user.name}`}
                      tooltip={`Edit ${user.name}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={PencilSquareIcon} />}
                      onClick={() => setEditDialog({ isOpen: true, user })}
                    />
                    <IconButton
                      label={`Reset password for ${user.name}`}
                      tooltip={`Reset password for ${user.name}`}
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={KeyIcon} />}
                      onClick={() => setResetDialog({ isOpen: true, user })}
                    />
                    <IconButton
                      label={
                        currentUser?.id != null && user.id != null && currentUser.id === user.id
                          ? `Delete ${user.name} (you cannot delete your own account)`
                          : `Delete ${user.name}`
                      }
                      tooltip={
                        currentUser?.id != null && user.id != null && currentUser.id === user.id
                          ? `Delete ${user.name} (you cannot delete your own account)`
                          : `Delete ${user.name}`
                      }
                      variant="ghost"
                      size="sm"
                      icon={<Icon icon={TrashIcon} />}
                      onClick={() => removeUser(user)}
                      isDisabled={currentUser?.id != null && user.id != null && currentUser.id === user.id}
                    />
                  </HStack>
                ),
              },
            ]}
            emptyState={
              <EmptyState
                title="No users yet"
                description="Create the first account to get started."
                isCompact
              />
            }
          />
        </VStack>
      </Section>
      {editDialog.isOpen ? (
        <UserDialog
          key={editDialog.user?.id ?? "new-user"}
          isOpen
          onOpenChange={(isOpen) => {
            if (!isOpen) setEditDialog({ isOpen: false, user: null });
          }}
          user={editDialog.user}
          currentUserId={currentUser?.id ?? null}
          onSaved={() => {
            setEditDialog({ isOpen: false, user: null });
            void usersState.refetch();
          }}
        />
      ) : null}
      {resetDialog.isOpen && resetDialog.user ? (
        <ResetPasswordDialog
          key={resetDialog.user.id ?? resetDialog.user.email}
          isOpen
          onOpenChange={(isOpen) => {
            if (!isOpen) setResetDialog({ isOpen: false, user: null });
          }}
          user={resetDialog.user}
          onSaved={() => {
            setResetDialog({ isOpen: false, user: null });
            void usersState.refetch();
          }}
        />
      ) : null}
    </VStack>
  );
}
