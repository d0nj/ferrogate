import { useState } from "react";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { Button } from "@astryxdesign/core/Button";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { NumberInput } from "@astryxdesign/core/NumberInput";
import { Section } from "@astryxdesign/core/Section";
import { Selector } from "@astryxdesign/core/Selector";
import { Switch } from "@astryxdesign/core/Switch";
import { Heading, Text } from "@astryxdesign/core/Text";

import { api } from "@/api/client";
import { queryKeys } from "@/api/queryKeys";
import type { RoutingPolicy, TierStrategy } from "@/api/types";
import { ErrorBanner } from "@/components/common/ErrorBanner";

const STRATEGIES: TierStrategy[] = ["weighted", "latency", "cost", "priority"];

const STRATEGY_HELP: Record<TierStrategy, string> = {
  weighted: "Weighted random: a higher weight tends to come first.",
  latency: "Lowest smoothed latency first; unmeasured providers are probed first.",
  cost: "Cheapest known unit cost first; unpriced models sort last.",
  priority: "Declared order, ignoring weight and latency.",
};

/** Route-resolution knobs that previously lived in Rust constants. */
export function RoutingPolicyPanel() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: [...queryKeys.routing, "policy"],
    queryFn: api.getRoutingPolicy,
  });

  const [edited, setEdited] = useState<RoutingPolicy | null>(null);
  const [savedAt, setSavedAt] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: (policy: RoutingPolicy) => api.saveRoutingPolicy(policy),
    onSuccess: ({ policy }) => {
      // The server returns the sanitised form; adopt it so the form stops
      // showing values the backend clamped.
      setEdited(policy);
      setSavedAt(new Date().toLocaleTimeString());
      void queryClient.invalidateQueries({ queryKey: queryKeys.routing });
    },
  });

  // Derived during render: local edits win until the query refetches, at which
  // point the server copy is authoritative again.
  const draft = edited ?? query.data ?? null;

  if (!draft) {
    return (
      <Section padding={0}>
        <Text color="secondary">
          {query.isLoading ? "Loading routing policy…" : "Policy unavailable."}
        </Text>
      </Section>
    );
  }

  const update = (next: Partial<RoutingPolicy>) =>
    setEdited({ ...draft, ...next });
  const retry = draft.retry_on;
  const dirty =
    !query.data || JSON.stringify(draft) !== JSON.stringify(query.data);

  return (
    <VStack gap={5}>
      <ErrorBanner
        message={query.error?.message ?? mutation.error?.message ?? null}
      />

      <Section dividers={["bottom"]}>
        <VStack gap={3}>
          <Heading level={3}>Health &amp; caching</Heading>
          <Text type="supporting">
            Applies to every provider. Changes take effect on the next request.
          </Text>
          <HStack gap={3} wrap="wrap">
            <NumberInput
              label="Failure threshold"
              value={draft.failure_threshold}
              min={1}
              max={100}
              isIntegerOnly
              onChange={(value) =>
                update({ failure_threshold: value ?? draft.failure_threshold })
              }
              description="Consecutive failures before a provider's circuit opens."
            />
            <NumberInput
              label="Cooldown (s)"
              value={draft.cooldown_seconds}
              min={1}
              max={3600}
              isIntegerOnly
              onChange={(value) =>
                update({ cooldown_seconds: value ?? draft.cooldown_seconds })
              }
              description="How long an open circuit stays out before probing."
            />
            <NumberInput
              label="Cache TTL (s)"
              value={draft.cache_ttl_seconds}
              min={1}
              max={300}
              isIntegerOnly
              onChange={(value) =>
                update({ cache_ttl_seconds: value ?? draft.cache_ttl_seconds })
              }
              description="How long a resolved alias is reused."
            />
          </HStack>
        </VStack>
      </Section>

      <Section dividers={["bottom"]}>
        <VStack gap={3}>
          <Heading level={3}>Selection</Heading>
          <Selector
            label="Default tier strategy"
            options={STRATEGIES}
            value={draft.strategy}
            onChange={(value) => update({ strategy: value as TierStrategy })}
            width="100%"
          />
          <Text type="supporting">{STRATEGY_HELP[draft.strategy]}</Text>
          <NumberInput
            label="Max attempts"
            value={draft.max_attempts}
            min={0}
            max={50}
            isIntegerOnly
            onChange={(value) => update({ max_attempts: value ?? 0 })}
            description="Targets tried before giving up. 0 walks the whole chain."
          />
          <NumberInput
            label="Key rotations"
            value={draft.key_rotations}
            min={0}
            max={10}
            isIntegerOnly
            onChange={(value) => update({ key_rotations: value ?? 0 })}
            description="Extra attempts rotating through one provider's credentials."
          />
        </VStack>
      </Section>

      <Section dividers={["bottom"]}>
        <VStack gap={3}>
          <Heading level={3}>Retry on</Heading>
          <Text type="supporting">
            A failure class that is off fails the request immediately instead of
            consuming the rest of the chain. Client errors are never retried.
          </Text>
          <Switch
            label="Rate limit (429)"
            value={retry.rate_limit}
            onChange={(checked) =>
              update({ retry_on: { ...retry, rate_limit: checked } })
            }
          />
          <Switch
            label="Server error (5xx)"
            value={retry.server_error}
            onChange={(checked) =>
              update({ retry_on: { ...retry, server_error: checked } })
            }
          />
          <Switch
            label="Transport failure"
            value={retry.transport}
            onChange={(checked) =>
              update({ retry_on: { ...retry, transport: checked } })
            }
          />
          <Switch
            label="Protocol error"
            value={retry.protocol}
            onChange={(checked) =>
              update({ retry_on: { ...retry, protocol: checked } })
            }
            labelTooltip="The provider answered in the wrong shape. Off by default."
          />
        </VStack>
      </Section>

      <HStack gap={3} vAlign="center">
        <Button
          label={mutation.isPending ? "Saving…" : "Save policy"}
          variant="primary"
          isDisabled={!dirty || mutation.isPending}
          onClick={() => void mutation.mutate(draft)}
        />
        <Button
          label="Reset"
          variant="secondary"
          isDisabled={!dirty || mutation.isPending}
          onClick={() => setEdited(query.data ?? draft)}
        />
        {savedAt ? (
          <Text type="supporting" color="secondary">
            Saved at {savedAt}
          </Text>
        ) : null}
      </HStack>
    </VStack>
  );
}
