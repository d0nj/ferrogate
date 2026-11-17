import { useState } from 'react';

import { Button } from '@astryxdesign/core/Button';
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog';
import { Grid } from '@astryxdesign/core/Grid';
import { HStack } from '@astryxdesign/core/HStack';
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout';
import { NumberInput } from '@astryxdesign/core/NumberInput';
import { Selector } from '@astryxdesign/core/Selector';
import { Switch } from '@astryxdesign/core/Switch';
import { TextArea } from '@astryxdesign/core/TextArea';
import { TextInput } from '@astryxdesign/core/TextInput';
import { VStack } from '@astryxdesign/core/VStack';

import { api } from '@/api/client';
import type {
  GuardrailAction,
  GuardrailDto,
  GuardrailPhase,
  GuardrailRuleType,
} from '@/api/types';
import { ErrorBanner } from '@/components/common/ErrorBanner';

const PHASES: GuardrailPhase[] = ['input', 'output'];
const RULE_TYPES: GuardrailRuleType[] = ['pii', 'secret', 'prompt_injection', 'regex'];
const ACTIONS: GuardrailAction[] = ['block', 'redact', 'flag'];

interface GuardrailDialogProps {
  isOpen: boolean;
  onOpenChange: (isOpen: boolean) => void;
  guardrail: GuardrailDto | null;
  onSaved: () => void;
}

export function GuardrailDialog({ isOpen, onOpenChange, guardrail, onSaved }: GuardrailDialogProps) {
  const [name, setName] = useState(guardrail?.name ?? '');
  const [phase, setPhase] = useState<GuardrailPhase>(
    (guardrail?.phase as GuardrailPhase) ?? 'input',
  );
  const [ruleType, setRuleType] = useState<GuardrailRuleType>(
    (guardrail?.rule_type as GuardrailRuleType) ?? 'regex',
  );
  const [pattern, setPattern] = useState(guardrail?.pattern ?? '');
  const [action, setAction] = useState<GuardrailAction>(
    (guardrail?.action as GuardrailAction) ?? 'block',
  );
  const [priority, setPriority] = useState<number | null>(guardrail?.priority ?? 0);
  const [isActive, setIsActive] = useState(guardrail?.is_active ?? true);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    const trimmedName = name.trim();
    if (!trimmedName) {
      setError('Name is required.');
      return;
    }
    const trimmedPattern = pattern.trim();
    if (ruleType === 'regex' && !trimmedPattern) {
      setError('Regex rules need a pattern.');
      return;
    }

    setError(null);
    try {
      const patternValue = ruleType === 'regex' ? trimmedPattern : trimmedPattern || null;
      if (guardrail) {
        await api.updateGuardrail(guardrail.id, {
          name: trimmedName,
          phase,
          rule_type: ruleType,
          pattern: patternValue,
          action,
          priority: priority ?? 0,
          is_active: isActive,
        });
      } else {
        await api.createGuardrail({
          name: trimmedName,
          phase,
          rule_type: ruleType,
          pattern: patternValue,
          action,
          priority: priority ?? 0,
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
            title={guardrail ? `Guardrail · ${guardrail.name}` : 'New guardrail'}
            onOpenChange={onOpenChange}
            hasDivider
          />
        }
        content={
          <LayoutContent>
            <VStack gap={3}>
              <ErrorBanner message={error} />
              <TextInput label="Name" value={name} onChange={setName} placeholder="pii-masking" />
              <Grid columns={{ minWidth: 220, max: 2 }} gap={3}>
                <Selector
                  label="Phase"
                  options={PHASES.map((value) => ({ value }))}
                  value={phase}
                  placeholder="Input"
                  onChange={(value) => setPhase((value as GuardrailPhase) ?? 'input')}
                />
                <Selector
                  label="Action"
                  options={ACTIONS.map((value) => ({ value }))}
                  value={action}
                  placeholder="Block"
                  onChange={(value) => setAction((value as GuardrailAction) ?? 'block')}
                />
              </Grid>
              <Selector
                label="Rule type"
                options={RULE_TYPES.map((value) => ({ value }))}
                value={ruleType}
                onChange={(value) => setRuleType(value as GuardrailRuleType)}
                hasSearch
              />
              <TextArea
                label="Pattern"
                value={pattern}
                onChange={setPattern}
                placeholder={ruleType === 'regex' ? '(?i)sk-[a-z0-9]{20,}' : 'Optional hint for this rule'}
                description={
                  ruleType === 'regex'
                    ? 'Regular expression evaluated against the request or response text.'
                    : 'Only regex rules evaluate a pattern; other types use built-in detectors.'
                }
              />
              <NumberInput
                label="Priority"
                value={priority}
                min={0}
                isIntegerOnly
                description="Lower runs first."
                onChange={setPriority}
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
                label={guardrail ? 'Save guardrail' : 'Create guardrail'}
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
