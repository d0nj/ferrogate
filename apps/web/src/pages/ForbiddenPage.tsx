import { Button } from '@astryxdesign/core/Button';
import { Heading } from '@astryxdesign/core/Heading';
import { HStack } from '@astryxdesign/core/HStack';
import { Section } from '@astryxdesign/core/Section';
import { Text } from '@astryxdesign/core/Text';
import { VStack } from '@astryxdesign/core/VStack';
import { useNavigate } from '@tanstack/react-router';

import { can } from '@/api/auth';
import type { Capability, CurrentUser, Role } from '@/api/auth';

const ROLES: Role[] = ['viewer', 'operator', 'admin', 'owner'];

function requiredRole(capability: Capability): Role {
  const stub = (role: Role): CurrentUser => ({
    id: null,
    email: '',
    name: '',
    role,
    is_active: true,
    created_at: null,
    last_login_at: null,
  });
  return ROLES.find((role) => can(stub(role), capability)) ?? 'owner';
}

interface ForbiddenPageProps {
  capability: Capability;
}

export function ForbiddenPage({ capability }: ForbiddenPageProps) {
  const navigate = useNavigate();
  const role = requiredRole(capability);

  return (
    <VStack gap={6}>
      <VStack gap={1}>
        <Heading level={1}>Access denied</Heading>
        <Text color="secondary">Your role does not include this area.</Text>
      </VStack>
      <Section padding={0}>
        <VStack gap={3}>
          <Heading level={3}>Requires the {role} role or higher</Heading>
          <Text type="supporting" size="sm">
            Ask an owner to grant access, or sign in with a different account.
          </Text>
          <HStack gap={2}>
            <Button
              label="Back to overview"
              variant="primary"
              onClick={() => {
                void navigate({ to: '/' });
              }}
            />
          </HStack>
        </VStack>
      </Section>
    </VStack>
  );
}
