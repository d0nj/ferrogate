import { Badge } from '@astryxdesign/core/Badge';
import { CodeBlock } from '@astryxdesign/core/CodeBlock';
import { Divider } from '@astryxdesign/core/Divider';
import { Drawer, DrawerHeader } from '@astryxdesign/core/Drawer';
import { HStack } from '@astryxdesign/core/HStack';
import { Text } from '@astryxdesign/core/Text';
import { VStack } from '@astryxdesign/core/VStack';

import type { RequestLogDto, RoutingAttempt } from '@/api/types';
import { formatMs, formatNumber, formatTime, formatUsd } from '@/utils/format';

interface LogInspectorDrawerProps {
  log: RequestLogDto | null;
  isRedacted: boolean;
  onOpenChange: (isOpen: boolean) => void;
}

function hasPrompts(log: RequestLogDto): boolean {
  return log.request_messages !== null && log.request_messages.length > 0;
}

function TraceStep({ attempt, index }: { attempt: RoutingAttempt; index: number }) {
  const isOk = attempt.status_code < 400;

  return (
    <HStack gap={3} vAlign="center" wrap="wrap">
      <Badge variant={isOk ? 'success' : 'error'} label={`${attempt.status_code}`} />
      <Text size="sm" weight="semibold">
        {index + 1}. {attempt.provider_name}
      </Text>
      <Text size="sm" color="secondary">
        {attempt.model}
      </Text>
      <Text size="sm" color="secondary" hasTabularNumbers>
        {formatMs(attempt.latency_ms)}
      </Text>
      {attempt.error ? (
        <Text size="sm" color="secondary">
          {attempt.error}
        </Text>
      ) : null}
    </HStack>
  );
}

export function LogInspectorDrawer({ log, isRedacted, onOpenChange }: LogInspectorDrawerProps) {
  return (
    <Drawer
      isOpen={log !== null}
      onOpenChange={onOpenChange}
      side="end"
      width="min(560px, calc(100vw - 2rem))"
      label="Request log inspector"
    >
      {log ? (
        <VStack gap={4}>
          <DrawerHeader
            title={log.requested_model}
            subtitle={formatTime(log.created_at)}
            onOpenChange={onOpenChange}
          />

          <HStack gap={2} wrap="wrap">
            <Badge
              variant={log.status_code < 400 ? 'success' : 'error'}
              label={`${log.status_code}`}
            />
            {log.is_streaming ? <Badge variant="blue" label="Streaming" /> : null}
            {log.cache_hit ? <Badge variant="cyan" label="Cache hit" /> : null}
            {log.fallback_count > 0 ? (
              <Badge variant="warning" label={`${log.fallback_count} fallbacks`} />
            ) : null}
          </HStack>

          <HStack gap={4} wrap="wrap">
            <VStack gap={0.5}>
              <Text size="2xs" color="secondary">
                Latency
              </Text>
              <Text size="sm" hasTabularNumbers>
                {formatMs(log.latency_ms)}
              </Text>
            </VStack>
            <VStack gap={0.5}>
              <Text size="2xs" color="secondary">
                TTFT
              </Text>
              <Text size="sm" hasTabularNumbers>
                {formatMs(log.ttft_ms)}
              </Text>
            </VStack>
            <VStack gap={0.5}>
              <Text size="2xs" color="secondary">
                Tokens
              </Text>
              <Text size="sm" hasTabularNumbers>
                {formatNumber(log.input_tokens)} in / {formatNumber(log.output_tokens)} out
              </Text>
            </VStack>
            <VStack gap={0.5}>
              <Text size="2xs" color="secondary">
                Cost
              </Text>
              <Text size="sm" hasTabularNumbers>
                {formatUsd(log.cost_usd)}
              </Text>
            </VStack>
          </HStack>

          <Divider />

          <VStack gap={2}>
            <Text weight="semibold" size="sm">
              Routing trace
            </Text>
            {log.routing_trace.length === 0 ? (
              <Text type="supporting" size="sm">
                No routing attempts recorded.
              </Text>
            ) : (
              log.routing_trace.map((attempt, index) => (
                <TraceStep key={`${attempt.provider_id}-${index}`} attempt={attempt} index={index} />
              ))
            )}
          </VStack>

          <Divider />

          <VStack gap={2}>
            <Text weight="semibold" size="sm">
              Request
            </Text>
            {isRedacted && hasPrompts(log) ? (
              <Text type="supporting" size="sm">
                Prompt content is redacted. Disable “Redact Prompts” to inspect messages.
              </Text>
            ) : hasPrompts(log) ? (
              <CodeBlock
                code={JSON.stringify(log.request_messages, null, 2)}
                language="json"
                hasCopyButton
                isCollapsible
                maxHeight={280}
              />
            ) : (
              <Text type="supporting" size="sm">
                No request messages captured.
              </Text>
            )}
          </VStack>

          <VStack gap={2}>
            <Text weight="semibold" size="sm">
              Response
            </Text>
            {isRedacted && log.response_text ? (
              <Text type="supporting" size="sm">
                Response content is redacted.
              </Text>
            ) : log.response_text ? (
              <CodeBlock code={log.response_text} hasCopyButton isCollapsible maxHeight={280} />
            ) : (
              <Text type="supporting" size="sm">
                No response body captured.
              </Text>
            )}
          </VStack>

          {log.error_message ? (
            <VStack gap={2}>
              <Text weight="semibold" size="sm">
                Error
              </Text>
              <CodeBlock code={log.error_message} hasCopyButton isWrapped />
            </VStack>
          ) : null}
        </VStack>
      ) : null}
    </Drawer>
  );
}
