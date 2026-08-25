<script lang="ts">
  import {
    Eye,
    EyeOff,
    Languages,
    Square,
    Trash2,
    Undo2,
    Upload,
  } from '@lucide/svelte';
  import type { Row } from '$lib/bindings';
  import ArmedButton from '$lib/components/armed-button.svelte';
  import type { ScopeActions } from '$lib/components/types';
  import { CLEAR_ARMED } from '$lib/wording';
  import { halted } from '$lib/project';

  let {
    scope,
    row,
    alive,
    acts,
  }: {
    scope: string;
    row: Row | undefined;
    alive: boolean;
    acts: ScopeActions;
  } = $props();

  const scopes = $derived([scope]);
  const excluded = $derived(row?.excluded ?? false);

  const ACT =
    'flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-xs text-ink-soft transition-colors enabled:hover:bg-sunken enabled:hover:text-ink disabled:text-ink-faint disabled:opacity-60';
</script>

<div class="flex shrink-0 flex-wrap items-center gap-1 px-4 pt-2 pb-3">
  <button
    class={ACT}
    disabled={halted()}
    onclick={() => acts.ontoggle(scopes, !excluded)}
  >
    {#if excluded}
      <Eye class="size-3.5 shrink-0" />
      Include in translation
    {:else}
      <EyeOff class="size-3.5 shrink-0" />
      Exclude from translation
    {/if}
  </button>

  <div class="ml-auto flex items-center gap-1">
    {#if alive}
      <button class={ACT} onclick={() => acts.onstop(scopes)}>
        <Square class="size-3.5 shrink-0" />
        Stop
      </button>
    {:else}
      <button
        class={ACT}
        disabled={halted() || excluded}
        onclick={() => acts.ontranslate(scopes)}
      >
        <Languages class="size-3.5 shrink-0" />
        Translate
      </button>
    {/if}

    <button
      class={ACT}
      disabled={halted() || excluded}
      onclick={() => acts.onexport(scopes)}
    >
      <Upload class="size-3.5 shrink-0" />
      Apply
    </button>

    <button
      class={ACT}
      disabled={halted() || !(row?.tally.applied ?? 0)}
      onclick={() => acts.onrevert(scopes)}
    >
      <Undo2 class="size-3.5 shrink-0" />
      Restore
    </button>

    {#key scope}
      <ArmedButton
        compact
        label="Clear"
        armedLabel={CLEAR_ARMED}
        Icon={Trash2}
        disabled={halted() || !(row?.tally.translated ?? 0)}
        onfire={() => acts.onclear(scopes)}
      />
    {/key}
  </div>
</div>
