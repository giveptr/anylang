import type { Provider } from '$lib/bindings';

type Preset = {
  id: string;
  label: string;
  kind: Provider;
  url?: string;
  firstModel?: string;
};

export const presets: Preset[] = [
  {
    id: 'gemini',
    label: 'Gemini',
    kind: 'gemini',
    firstModel: 'gemini-3.5-flash-lite',
  },
  {
    id: 'vertex',
    label: 'Vertex AI',
    kind: 'vertex',
    firstModel: 'gemini-3.5-flash-lite',
  },
  {
    id: 'claude',
    label: 'Claude',
    kind: 'claude',
    firstModel: 'claude-haiku-4-5',
  },
  {
    id: 'openai',
    label: 'OpenAI',
    kind: 'compatible',
    url: 'https://api.openai.com/v1',
    firstModel: 'gpt-5.6-luna',
  },
  {
    id: 'openrouter',
    label: 'OpenRouter',
    kind: 'compatible',
    url: 'https://openrouter.ai/api/v1',
  },
  {
    id: 'deepseek',
    label: 'DeepSeek',
    kind: 'compatible',
    url: 'https://api.deepseek.com/v1',
  },
  { id: 'custom', label: 'Other OpenAI-compatible', kind: 'compatible' },
];

export const firstModel = (preset: string): string =>
  presets.find((one) => one.id === preset)?.firstModel ?? '';

export const presetOf = (using: Provider, baseUrl: string): string => {
  if (using !== 'compatible') return using;
  return presets.find((one) => one.url && one.url === baseUrl)?.id ?? 'custom';
};
