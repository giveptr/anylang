<script lang="ts">
  import {
    commands,
    type Era,
    type Fidelity,
    type Mood,
    type Project,
    type Register,
  } from '$lib/bindings';
  import { caught } from '$lib/save';
  import { app } from '$lib/app.svelte';
  import Picker from '$lib/components/picker.svelte';
  import TagPicker from '$lib/components/tag-picker.svelte';

  let { project }: { project: Project } = $props();

  let preview = $state('');

  $effect(() => {
    const asked = $state.snapshot(project);
    if (!app.gameDir) return;

    const timer = setTimeout(async () => {
      const built = await caught(() =>
        commands.previewPrompt(app.gameDir, asked),
      );
      if (JSON.stringify(asked) !== JSON.stringify($state.snapshot(project)))
        return;

      preview = built.status === 'ok' ? built.data : built.error;
    }, 200);

    return () => clearTimeout(timer);
  });

  const NOTES = `Keep -san and -chan instead of dropping them.
Write みさき as Misaki everywhere.
Misaki speaks casually with everyone. Her butler stays formal.`;

  type Told = { label: string; hint: string };

  const eras: Record<Exclude<Era, 'any'>, Told> = {
    ancient: { label: 'Ancient', hint: 'the ancient world' },
    medieval: { label: 'Medieval', hint: 'castles and swords' },
    earlyModern: { label: 'Early modern', hint: 'a few centuries back' },
    victorian: {
      label: 'Nineteenth century',
      hint: 'the Victorian age, the Meiji era',
    },
    earlyTwentieth: {
      label: 'Early twentieth century',
      hint: 'the world wars and before',
    },
    lateTwentieth: {
      label: 'Late twentieth century',
      hint: 'the postwar decades',
    },
    modern: { label: 'Modern', hint: 'the present day' },
    nearFuture: { label: 'Near future', hint: 'a short way ahead' },
    farFuture: { label: 'Far future', hint: 'long from now' },
  };

  const fidelities: Record<Fidelity, Told> = {
    balanced: { label: 'Balanced', hint: 'meaning over wording' },
    free: { label: 'Free', hint: 'rewrite for the best flow' },
    literal: { label: 'Literal', hint: 'stay close to the wording' },
  };

  const registers: Record<Exclude<Register, 'any'>, Told> = {
    coarse: { label: 'Coarse', hint: 'foul-mouthed' },
    casual: { label: 'Casual', hint: 'loose and everyday' },
    formal: { label: 'Formal', hint: 'measured and polite' },
    elevated: { label: 'Elevated', hint: 'grand and lofty' },
  };

  const optioned = (told: Record<string, Told>) =>
    Object.entries(told).map(([value, one]) => ({ value, ...one }));

  const singles = [
    {
      key: 'era' as const,
      label: 'Era',
      clearable: true,
      options: optioned(eras),
    },
    {
      key: 'fidelity' as const,
      label: 'Fidelity',
      clearable: false,
      options: optioned(fidelities),
    },
    {
      key: 'register' as const,
      label: 'Register',
      clearable: true,
      options: optioned(registers),
    },
  ];

  const genre = {
    key: 'genres' as const,
    label: 'Genre',
    options: [
      { value: 'adult', label: 'Adult' },
      { value: 'comedy', label: 'Comedy' },
      { value: 'crime', label: 'Crime' },
      { value: 'cyberpunk', label: 'Cyberpunk' },
      { value: 'dystopian', label: 'Dystopian' },
      { value: 'fairyTale', label: 'Fairy tale' },
      { value: 'fantasy', label: 'Fantasy' },
      { value: 'historical', label: 'Historical' },
      { value: 'horror', label: 'Horror' },
      { value: 'isekai', label: 'Isekai' },
      { value: 'mecha', label: 'Mecha' },
      { value: 'military', label: 'Military' },
      { value: 'mystery', label: 'Mystery' },
      { value: 'postApocalyptic', label: 'Post-apocalyptic' },
      { value: 'romance', label: 'Romance' },
      { value: 'schoolLife', label: 'School life' },
      { value: 'sciFi', label: 'Sci-fi' },
      { value: 'sliceOfLife', label: 'Slice of life' },
      { value: 'steampunk', label: 'Steampunk' },
      { value: 'supernatural', label: 'Supernatural' },
      { value: 'thriller', label: 'Thriller' },
      { value: 'western', label: 'Western' },
    ],
  };

  const moods: Record<Mood, string> = {
    comic: 'Comic',
    cute: 'Cute',
    dark: 'Dark',
    deadpan: 'Deadpan',
    dramatic: 'Dramatic',
    epic: 'Epic',
    explicit: 'Explicit',
    melancholic: 'Melancholic',
    playful: 'Playful',
    sarcastic: 'Sarcastic',
    tense: 'Tense',
    unsettling: 'Unsettling',
    warm: 'Warm',
    witty: 'Witty',
  };

  const voice = {
    key: 'voices' as const,
    label: 'Mood',
    options: Object.entries(moods).map(([value, label]) => ({ value, label })),
  };
</script>

{#snippet chips(axis: typeof genre | typeof voice)}
  <div class="flex flex-col gap-1.5">
    <span class="text-sm font-medium">{axis.label}</span>
    <TagPicker
      bind:values={project.style[axis.key]}
      options={axis.options}
      placeholder="No preference"
    />
  </div>
{/snippet}

<div
  class="grid gap-8 lg:grid-cols-[minmax(0,26rem)_minmax(0,1fr)] lg:items-start"
>
  <div class="flex flex-col gap-6">
    {@render chips(genre)}

    {#each singles as axis (axis.key)}
      <div class="flex flex-col gap-1.5">
        <span class="text-sm font-medium">{axis.label}</span>
        <Picker
          value={project.style[axis.key] === 'any'
            ? ''
            : (project.style[axis.key] ?? '')}
          options={axis.options}
          placeholder={axis.clearable ? 'No preference' : ''}
          searchable={false}
          clearable={axis.clearable}
          onpick={(chosen) =>
            Object.assign(project.style, { [axis.key]: chosen || 'any' })}
        />
      </div>
    {/each}

    {@render chips(voice)}

    <div class="flex flex-col gap-1.5">
      <span class="text-sm font-medium">Notes for the model</span>
      <textarea
        rows="8"
        bind:value={project.style.notes}
        placeholder={NOTES}
        class="w-full resize-y rounded-lg bg-surface px-3 py-2 font-mono text-xs leading-relaxed ring-1 ring-edge placeholder:text-ink-faint focus:ring-accent"
      ></textarea>
    </div>
  </div>

  <aside
    class="flex min-w-0 flex-col gap-1.5 border-t border-line pt-5 lg:sticky lg:top-0 lg:border-t-0 lg:border-l lg:pt-0 lg:pl-8"
  >
    <span class="text-sm font-medium">What the model is told</span>
    <pre
      class="overflow-auto font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-ink-soft lg:max-h-[calc(100vh-9rem)]">{preview}</pre>
  </aside>
</div>
