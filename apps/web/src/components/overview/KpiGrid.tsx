import { Card } from '@astryxdesign/core/Card'
import { Grid } from '@astryxdesign/core/Grid'
import { Heading } from '@astryxdesign/core/Heading'
import { VStack } from '@astryxdesign/core/Stack'
import { Text } from '@astryxdesign/core/Text'

import type { OverviewKpis } from '@/api/types'
import { formatMs, formatNumber, formatUsd } from '@/utils/format'

interface KpiRow {
  label: string
  value: string
  detail: string
}

function toRows(kpis: OverviewKpis | null): KpiRow[] {
  if (!kpis) {
    return [
      { label: 'Requests', value: '—', detail: 'RPM —' },
      { label: 'Spend', value: '—', detail: 'No traffic' },
      { label: 'TTFT', value: '—', detail: 'p50 — · p95 —' },
      { label: 'Errors', value: '—', detail: 'Fallback —' },
    ]
  }

  return [
    {
      label: 'Requests',
      value: formatNumber(kpis.total_requests),
      detail: `${kpis.rpm.toFixed(1)} RPM`,
    },
    {
      label: 'Spend',
      value: formatUsd(kpis.total_spend_usd),
      detail: `Cache hits ${kpis.cache_hit_rate_pct.toFixed(1)}%`,
    },
    {
      label: 'TTFT',
      value: formatMs(kpis.ttft_p50_ms),
      detail: `p50 ${formatMs(kpis.ttft_p50_ms)} · p95 ${formatMs(kpis.ttft_p95_ms)}`,
    },
    {
      label: 'Errors',
      value: `${kpis.error_rate_pct.toFixed(2)}%`,
      detail: `Fallback ${kpis.fallback_rate_pct.toFixed(2)}%`,
    },
  ]
}

export function KpiGrid({ kpis }: { kpis: OverviewKpis | null }) {
  return (
    <Grid columns={{ minWidth: 160, max: 4 }} gap={3}>
      {toRows(kpis).map((row) => (
        <Card key={row.label}>
          <VStack gap={1}>
            <Text color="secondary">{row.label}</Text>
            <Heading level={2}>{row.value}</Heading>
            <Text type="supporting" hasTabularNumbers maxLines={1}>
              {row.detail}
            </Text>
          </VStack>
        </Card>
      ))}
    </Grid>
  )
}
