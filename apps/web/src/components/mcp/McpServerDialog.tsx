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
import type { McpServerDto } from '@/api/types';
import { ErrorBanner } from '@/components/common/ErrorBanner';
import { MultiSelector } from '@astryxdesign/core/MultiSelector';

const TRANSPORTS = ['http', 'sse'];

interface McpServerDialogProps {
  isOpen: boolean;
  onOpenChange: (isOpen: boolean) => void;
  server: McpServerDto | null;
  onSaved: () => void;
}

export function McpServerDialog({ isOpen, onOpenChange, server, onSaved }: McpServerDialogProps) {
  const [name, setName] = useState(server?.name ?? '');
  const [url, setUrl] = useState(server?.url ?? '');
  const [transport, setTransport] = useState(server?.transport ?? 'http');
  const [allowedTools, setAllowedTools] = useState<string[]>(server?.allowed_tools ?? []);
  const [isActive, setIsActive] = useState(server?.is_active ?? true);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    const trimmedName = name.trim();
    const trimmedUrl = url.trim();
    if (!trimmedName || !trimmedUrl) {
      setError('Name and URL are required.');
      return;
    }

    setError(null);
    try {
      const tools = allowedTools;
      if (server) {
        await api.updateMcpServer(server.id, {
          name: trimmedName,
          url: trimmedUrl,
          transport,
          allowed_tools: tools,
          is_active: isActive,
        });
      } else {
        await api.createMcpServer({
          name: trimmedName,
          url: trimmedUrl,
          transport,
          allowed_tools: tools,
          is_active: isActive,
        });
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
            title={server ? `MCP server · ${server.name}` : 'New MCP server'}
            onOpenChange={onOpenChange}
            hasDivider
          />
        }
        content={
          <LayoutContent>
            <VStack gap={3}>
              <ErrorBanner message={error} />
              <TextInput
                label="Name"
                value={name}
                onChange={setName}
                placeholder="docs-search"
              />
              <TextInput
                label="URL"
                value={url}
                onChange={setUrl}
                placeholder="https://mcp.example.com/rpc"
              />
              <Selector
                label="Transport"
                options={TRANSPORTS.map((value) => ({ value }))}
                value={transport}
                onChange={(value) => setTransport(value)}
              />
              <MultiSelector
                label="Allowed tools"
                options={[...new Set([...(server?.allowed_tools ?? []), ...allowedTools])]}
                value={allowedTools}
                onChange={setAllowedTools}
                placeholder="Empty allows every tool"
                hasSearch
                hasCreate
                hasClear
              />
              <Switch label="Active" value={isActive} onChange={setIsActive} />
            </VStack>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider>
            <HStack gap={2} hAlign="end" width="100%">
              <Button label="Cancel" variant="ghost" onClick={() => onOpenChange(false)} />
              <Button
                label={server ? 'Save server' : 'Create server'}
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
