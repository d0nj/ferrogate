import { useState } from 'react';

import { Button } from '@astryxdesign/core/Button';
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog';
import { HStack } from '@astryxdesign/core/HStack';
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout';
import { Selector } from '@astryxdesign/core/Selector';
import { Switch } from '@astryxdesign/core/Switch';
import { TextInput } from '@astryxdesign/core/TextInput';
import { VStack } from '@astryxdesign/core/VStack';

import { api } from '@/api/client';
import type { Role } from '@/api/auth';
import type { UserDto } from '@/api/types';
import { ErrorBanner } from '@/components/common/ErrorBanner';

const ROLES: Role[] = ['owner', 'admin', 'operator', 'viewer'];

interface UserDialogProps {
  isOpen: boolean;
  onOpenChange: (isOpen: boolean) => void;
  user: UserDto | null;
  currentUserId: string | null;
  onSaved: () => void;
}

export function UserDialog({ isOpen, onOpenChange, user, currentUserId, onSaved }: UserDialogProps) {
  const [name, setName] = useState(user?.name ?? '');
  const [email, setEmail] = useState(user?.email ?? '');
  const [password, setPassword] = useState('');
  const [role, setRole] = useState<Role>(user?.role ?? 'viewer');
  const [isActive, setIsActive] = useState(user?.is_active ?? true);
  const [error, setError] = useState<string | null>(null);

  const isSelf = currentUserId != null && user?.id != null && currentUserId === user.id;

  async function save() {
    const trimmedName = name.trim();
    const trimmedEmail = email.trim();
    if (!trimmedName || !trimmedEmail) {
      setError('Name and email are required.');
      return;
    }

    setError(null);
    try {
      if (user) {
        if (!user.id) {
          setError('This account cannot be edited.');
          return;
        }
        await api.updateUser(user.id, {
          name: trimmedName,
          email: trimmedEmail,
          role,
          is_active: isActive,
        });
      } else {
        if (password.length < 12) {
          setError('Password must be at least 12 characters.');
          return;
        }
        await api.createUser({ name: trimmedName, email: trimmedEmail, password, role });
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
            title={user ? `User · ${user.name}` : 'New user'}
            onOpenChange={onOpenChange}
            hasDivider
          />
        }
        content={
          <LayoutContent>
            <VStack gap={3}>
              <ErrorBanner message={error} />
              <TextInput label="Name" value={name} onChange={setName} placeholder="Ada Lovelace" />
              <TextInput
                label="Email"
                type="email"
                value={email}
                onChange={setEmail}
                placeholder="ada@example.com"
                autoComplete="username"
              />
              {user ? null : (
                <TextInput
                  label="Password"
                  type="password"
                  value={password}
                  onChange={setPassword}
                  placeholder="••••••••"
                  description="At least 12 characters."
                  autoComplete="new-password"
                />
              )}
              <Selector
                label="Role"
                options={ROLES.map((value) => ({ value }))}
                value={role}
                onChange={(value) => setRole(value as Role)}
                isDisabled={isSelf}
                disabledMessage={isSelf ? 'You cannot change the role of your own account.' : undefined}
              />
              {user ? (
                <Switch
                  label="Active"
                  value={isActive}
                  onChange={setIsActive}
                  isDisabled={isSelf}
                  disabledMessage={isSelf ? 'You cannot deactivate your own account.' : undefined}
                />
              ) : null}
            </VStack>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider>
            <HStack gap={2} hAlign="end" width="100%">
              <Button label="Cancel" variant="ghost" onClick={() => onOpenChange(false)} />
              <Button
                label={user ? 'Save changes' : 'Create user'}
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

interface ResetPasswordDialogProps {
  isOpen: boolean;
  onOpenChange: (isOpen: boolean) => void;
  user: UserDto;
  onSaved: () => void;
}

export function ResetPasswordDialog({ isOpen, onOpenChange, user, onSaved }: ResetPasswordDialogProps) {
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);

  async function save() {
    if (password.length < 12) {
      setError('Password must be at least 12 characters.');
      return;
    }
    if (!user.id) {
      setError('This account has no password to reset.');
      return;
    }

    setError(null);
    try {
      await api.resetUserPassword(user.id, password);
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
      width="min(480px, calc(100vw - 2rem))"
      maxHeight="85dvh"
    >
      <Layout
        header={
          <DialogHeader
            title={`Reset password · ${user.name}`}
            onOpenChange={onOpenChange}
            hasDivider
          />
        }
        content={
          <LayoutContent>
            <VStack gap={3}>
              <ErrorBanner message={error} />
              <TextInput
                label="New password"
                type="password"
                value={password}
                onChange={setPassword}
                placeholder="••••••••"
                description="At least 12 characters."
                autoComplete="new-password"
              />
            </VStack>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider>
            <HStack gap={2} hAlign="end" width="100%">
              <Button label="Cancel" variant="ghost" onClick={() => onOpenChange(false)} />
              <Button label="Reset password" variant="primary" onClick={save} />
            </HStack>
          </LayoutFooter>
        }
      />
    </Dialog>
  );
}
