import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Button } from "@astryxdesign/core/Button";
import {
  ChatComposer,
  ChatComposerInput,
  ChatLayout,
  ChatMessage,
  ChatMessageBubble,
  ChatMessageList,
} from "@astryxdesign/core/Chat";
import { Drawer, DrawerHeader } from "@astryxdesign/core/Drawer";
import { DropdownMenu } from "@astryxdesign/core/DropdownMenu";
import { Icon } from "@astryxdesign/core/Icon";
import { HStack, Stack, VStack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { Cog6ToothIcon } from "@heroicons/react/24/outline";

import { api, streamChatCompletion } from "@/api/client";
import { queryKeys } from "@/api/queryKeys";
import type { ChatMessagePayload, StreamStats } from "@/api/types";
import { usePageAction } from "@/components/common/use-page-action";
import {
  PlaygroundControls,
  type PlaygroundSettings,
} from "@/components/playground/PlaygroundControls";
import {
  PromptTemplatePicker,
  type TemplatePickerValue,
} from "@/components/playground/PromptTemplatePicker";
import { useMediaQuery } from "@astryxdesign/core/hooks";

// The Astryx `md` breakpoint, the same one AppShell uses for MobileNav.
const NARROW_QUERY = "(max-width: 767px)";

interface Turn {
  role: "user" | "assistant";
  content: string;
}

const INITIAL_SETTINGS: PlaygroundSettings = {
  apiKey: "",
  model: "default",
  systemPrompt: "",
  temperature: 0.7,
  maxTokens: 1024,
};

function toPayloads(turns: Turn[], systemPrompt: string): ChatMessagePayload[] {
  const messages: ChatMessagePayload[] = [];
  if (systemPrompt.trim().length > 0) {
    messages.push({ role: "system", content: systemPrompt });
  }
  for (const turn of turns) {
    messages.push({ role: turn.role, content: turn.content });
  }
  return messages;
}

export function PlaygroundPage() {
  const [settings, setSettings] = useState<PlaygroundSettings>(INITIAL_SETTINGS);
  const [templates, setTemplates] = useState<TemplatePickerValue>({
    selection: null,
    variables: {},
  });
  const [turns, setTurns] = useState<Turn[]>([]);
  const [draft, setDraft] = useState("");
  const [stats, setStats] = useState<StreamStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isStreaming, setIsStreaming] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isMintingKey, setIsMintingKey] = useState(false);
  const abortRef = useRef<AbortController | null>(null);
  const isNarrow = useMediaQuery(NARROW_QUERY);

  const routesQuery = useQuery({
    queryKey: queryKeys.routes,
    queryFn: api.listRoutes,
  });
  const providersQuery = useQuery({
    queryKey: queryKeys.providers,
    queryFn: api.listProviders,
  });

  // Only an alias resolves to a routing target. A provider's upstream model ids
  // are what a route points *at*; offering them unqualified produced names the
  // proxy answers with "no routing target found".
  const models = useMemo(
    () =>
      [...new Set((routesQuery.data ?? []).map((route) => route.alias))].sort(),
    [routesQuery.data],
  );

  // `provider/model` resolves without a route, but only where the provider is
  // active and still lists the model — the same gate the resolver uses.
  const directModels = useMemo(
    () =>
      (providersQuery.data ?? [])
        .filter((provider) => provider.is_active && provider.models.length > 0)
        .map((provider) => ({ provider: provider.name, models: provider.models }))
        .sort((a, b) => a.provider.localeCompare(b.provider)),
    [providersQuery.data],
  );

  const isModelsLoading = routesQuery.isPending || providersQuery.isPending;
  const modelsError =
    routesQuery.error?.message ?? providersQuery.error?.message ?? null;
  const statusMessage = error ?? modelsError;

  // An alias that disappeared must not stay selected. Derived, not stored: a
  // reset effect would re-render the whole chat on every route refetch.
  const activeModel =
    settings.model && models.includes(settings.model)
      ? settings.model
      : models.includes("default")
        ? "default"
        : models[0] || "";

  useEffect(() => () => abortRef.current?.abort(), []);

  const updateSettings = useCallback((next: Partial<PlaygroundSettings>) => {
    setSettings((current) => ({ ...current, ...next }));
  }, []);

  // Mint once per session so repeated sends reuse the same key.
  const keyRef = useRef<string | null>(null);

  const mintPlaygroundKey = useCallback(async () => {
    if (keyRef.current) {
      updateSettings({ apiKey: keyRef.current });
      return;
    }
    setIsMintingKey(true);
    setError(null);
    try {
      const { plaintext_key } = await api.createPlaygroundKey();
      keyRef.current = plaintext_key;
      updateSettings({ apiKey: plaintext_key });
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not create a playground key",
      );
    } finally {
      setIsMintingKey(false);
    }
  }, [updateSettings]);

  const send = useCallback(
    async (content: string) => {
      if (isStreaming || content.trim().length === 0) return;

      const userTurn: Turn = { role: "user", content };
      const history = [...turns, userTurn];
      setTurns([...history, { role: "assistant", content: "" }]);
      setDraft("");
      setError(null);
      setStats(null);

      const controller = new AbortController();
      abortRef.current = controller;
      setIsStreaming(true);

      const writeAssistant = (text: string) =>
        setTurns((current) => {
          const next = [...current];
          const last = next[next.length - 1];
          if (last && last.role === "assistant") {
            next[next.length - 1] = { ...last, content: text };
          }
          return next;
        });

      try {
        await streamChatCompletion({
          apiKey: settings.apiKey,
          model: activeModel,
          messages: toPayloads(history, settings.systemPrompt),
          temperature: settings.temperature,
          maxTokens: settings.maxTokens,
          signal: controller.signal,
          template: templates.selection?.slug,
          templateVersion: templates.selection?.version ?? undefined,
          templateVariables: templates.variables,
          onDelta: writeAssistant,
          onStats: setStats,
        });
      } catch (cause) {
        if (controller.signal.aborted) return;
        setError(cause instanceof Error ? cause.message : "Stream failed");
      } finally {
        abortRef.current = null;
        setIsStreaming(false);
      }
    },
    [isStreaming, settings, activeModel, templates, turns],
  );

  const stop = useCallback(() => abortRef.current?.abort(), []);

  const clear = useCallback(() => {
    abortRef.current?.abort();
    setTurns([]);
    setStats(null);
    setError(null);
  }, []);

  usePageAction(
    () => (
      <Button
        label="New chat"
        variant="secondary"
        isDisabled={isStreaming || turns.length === 0}
        onClick={clear}
      />
    ),
    [isStreaming, turns.length, clear],
  );

  const settingsPanel = (
    <PlaygroundControls
      settings={settings}
      onChange={updateSettings}
      isStreaming={isStreaming}
      stats={stats}
      isTemplateSelected={templates.selection !== null}
      onUseSavedKey={() => void mintPlaygroundKey()}
      isMintingKey={isMintingKey}
    />
  );

  return (
    <>
      <ChatLayout
        emptyState={
          <VStack gap={2} vAlign="center" hAlign="center">
            <Text type="large" as="h2">
              Test routing before you ship
            </Text>
            <Text color="secondary">
              Send a prompt through the real proxy and watch tokens, cost and
              latency in the settings panel.
            </Text>
          </VStack>
        }
        composer={
          <HStack width="100%" hAlign="center">
            <Stack width="100%" maxWidth={768}>
              <ChatComposer
            value={draft}
            onChange={setDraft}
            onSubmit={(value) => void send(value)}
            onStop={stop}
            isStopShown={isStreaming}
            placeholder={
              activeModel ? `Message ${activeModel}` : "Type a message"
            }
            input={
              <ChatComposerInput
                value={draft}
                onChange={setDraft}
                label="Message input"
                maxRows={10}
                placeholder={
                  activeModel ? `Message ${activeModel}` : "Type a message"
                }
                style={{ minHeight: 76 }}
              />
            }
            footerActions={
              // Model picker and settings sit together: both are composer-level
              // decisions, and the picker chooses the model in place.
              <HStack gap={2} vAlign="center">
              <DropdownMenu
                button={{
                  label: isModelsLoading
                    ? "Loading models"
                    : activeModel || "Select model",
                  variant: "secondary",
                  size: "sm",
                }}
                menuWidth={280}
                items={[
                  {
                    type: "section",
                    id: "aliases",
                    title: "Routing aliases",
                    items: models.map((alias) => ({
                      label: alias,
                      onClick: () => updateSettings({ model: alias }),
                    })),
                  },
                  ...directModels.map((group) => ({
                    type: "section" as const,
                    id: group.provider,
                    title: group.provider,
                    items: group.models
                      .filter((model) => !models.includes(model))
                      .map((model) => ({
                        label: model,
                        onClick: () =>
                          updateSettings({ model: `${group.provider}/${model}` }),
                      })),
                  })),
                ]}
              />
              <Button
                label="Settings"
                variant="ghost"
                size="sm"
                isIconOnly
                icon={<Icon icon={Cog6ToothIcon} size="sm" />}
                onClick={() => setIsSettingsOpen(true)}
              />
              </HStack>
            }
            status={statusMessage ? { type: "error", message: statusMessage } : undefined}
          />
              </Stack>
            </HStack>
        }
      >
        {turns.length > 0 ? (
          <ChatMessageList isStreaming={isStreaming}>
            {turns.map((turn, index) => (
              <ChatMessage key={index} sender={turn.role}>
                <ChatMessageBubble>{turn.content || " "}</ChatMessageBubble>
              </ChatMessage>
            ))}
          </ChatMessageList>
        ) : null}
      </ChatLayout>

      <Drawer
        isOpen={isSettingsOpen}
        onOpenChange={setIsSettingsOpen}
        label="Playground settings"
        side="end"
        width={isNarrow ? "100%" : 400}
      >
        <DrawerHeader
          title="Settings"
          subtitle={activeModel || undefined}
          onOpenChange={setIsSettingsOpen}
        />
        <VStack gap={5} paddingInline={4} paddingBlock={3}>
          {settingsPanel}
          <PromptTemplatePicker value={templates} onChange={setTemplates} />
        </VStack>
      </Drawer>
    </>
  );
}
