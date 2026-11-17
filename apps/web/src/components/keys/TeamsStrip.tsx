import { HStack } from '@astryxdesign/core/HStack';
import { Text } from '@astryxdesign/core/Text';
import { VStack } from '@astryxdesign/core/VStack';

import { api } from '@/api/client';
import { queryKeys } from '@/api/queryKeys';
import { formatUsd } from '@/utils/format';
import { useQuery } from '@tanstack/react-query';

export function TeamsStrip() {
  const teamsState = useQuery({
    queryKey: queryKeys.teams,
    queryFn: api.listTeams,
  });
  const teams = teamsState.data ?? [];

  if (teams.length === 0) return null;

  return (
    <HStack gap={4} wrap="wrap">
      {teams.map((team) => (
        <VStack key={team.name} gap={0.5}>
          <Text weight="semibold">{team.name}</Text>
          <Text type="supporting" size="sm">
            {team.keys === 1 ? '1 key' : `${team.keys} keys`} · {formatUsd(team.spend_usd)}
          </Text>
        </VStack>
      ))}
    </HStack>
  );
}
