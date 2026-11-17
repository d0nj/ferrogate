import { useEffect, useState } from "react";

import { Heading, Text } from "@astryxdesign/core/Text";
import { Section } from "@astryxdesign/core/Section";
import { Switch } from "@astryxdesign/core/Switch";
import { VStack } from "@astryxdesign/core/VStack";

import { api } from "@/api/client";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { CachePanel } from "@/components/settings/CachePanel";
import { SemanticCachePanel } from "@/components/settings/SemanticCachePanel";

function RedactPromptsSetting() {
  const [value, setValue] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let isCurrent = true;
    api
      .getSettings()
      .then((settings) => {
        if (isCurrent) setValue(settings.redact_prompts);
      })
      .catch((cause: unknown) => {
        if (isCurrent)
          setError(cause instanceof Error ? cause.message : String(cause));
      });
    return () => {
      isCurrent = false;
    };
  }, []);

  async function toggle(next: boolean) {
    setValue(next);
    setError(null);
    try {
      await api.putSettings({ redact_prompts: next });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setValue(!next);
    }
  }

  return (
    <Section padding={0}>
      <VStack gap={3}>
        <VStack gap={0.5}>
          <Heading level={3}>Prompt privacy</Heading>
          <Text type="supporting" size="sm">
            Strip message contents from stored request logs.
          </Text>
        </VStack>
        <ErrorBanner message={error} />
        <Switch
          label="Redact prompts in logs"
          value={value ?? false}
          onChange={toggle}
          isDisabled={value === null}
        />
      </VStack>
    </Section>
  );
}

export function SettingsPage() {
  return (
    <VStack gap={4}>
      <CachePanel />
      <SemanticCachePanel />
      <RedactPromptsSetting />
    </VStack>
  );
}
