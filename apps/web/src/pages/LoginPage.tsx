import { useState } from "react";

import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Center } from "@astryxdesign/core/Center";
import { HStack } from "@astryxdesign/core/Stack";
import { Heading, Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { VStack } from "@astryxdesign/core/VStack";

import { loginWithCredentials } from "@/api/auth";
import { ErrorBanner } from "@/components/common/ErrorBanner";
import { FerroGateLogo } from "@/components/common/FerroGateLogo";

export function LoginPage() {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);

  async function submit() {
    if (!email.trim()) {
      setError("Enter your email address.");
      return;
    }
    if (!password) {
      setError("Enter your password.");
      return;
    }
    setError(null);
    setIsSubmitting(true);
    try {
      await loginWithCredentials(email.trim(), password);
      const redirect = new URLSearchParams(window.location.search).get("redirect");
      const target =
        redirect && redirect.startsWith("/") && !redirect.startsWith("//") ? redirect : "/";
      window.location.assign(target);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setIsSubmitting(false);
    }
  }

  return (
    <Center minHeight="100dvh">
      <Card maxWidth={400} width="100%" padding={5}>
        <VStack gap={4}>
          <VStack gap={1}>
            <HStack gap={2} vAlign="center">
              <FerroGateLogo size={28} />
              <Heading level={2}>FerroGate</Heading>
            </HStack>
            <Text color="secondary">Sign in to open the control plane.</Text>
          </VStack>
          <ErrorBanner message={error} />
          <TextInput
            label="Email"
            type="email"
            value={email}
            onChange={setEmail}
            placeholder="admin@ferrogate.local"
            hasAutoFocus
            onEnter={submit}
            autoComplete="username"
          />
          <TextInput
            label="Password"
            type="password"
            value={password}
            onChange={setPassword}
            placeholder="••••••••"
            onEnter={submit}
            autoComplete="current-password"
          />
          <Button
            label="Sign in"
            variant="primary"
            onClick={submit}
            isLoading={isSubmitting}
          />
        </VStack>
      </Card>
    </Center>
  );
}
