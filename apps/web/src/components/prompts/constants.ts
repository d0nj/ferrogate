import type { PromptVariable } from '@/api/types';

export function parsePromptVariables(text: string): PromptVariable[] {
  const variables: PromptVariable[] = [];
  for (const line of text.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const separator = trimmed.indexOf('=');
    if (separator === -1) {
      variables.push({ name: trimmed });
    } else {
      const name = trimmed.slice(0, separator).trim();
      const fallback = trimmed.slice(separator + 1).trim();
      if (!name) continue;
      variables.push(fallback ? { name, default: fallback } : { name });
    }
  }
  return variables;
}

export function formatPromptVariables(variables: PromptVariable[]): string {
  return variables
    .map((variable) =>
      variable.default ? `${variable.name}=${variable.default}` : variable.name,
    )
    .join('\n');
}
