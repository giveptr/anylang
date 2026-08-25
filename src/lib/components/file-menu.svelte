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
  import {
    APPLY,
    CLEAR_ARMED,
    CLEAR_WARNING,
    RESTORE,
    TRANSLATE,
  } from '$lib/wording';
  import { halted } from '$lib/project';
  import ArmedButton from '$lib/components/armed-button.svelte';
  import ContextMenu from '$lib/components/context-menu.svelte';
  import MenuItem from '$lib/components/menu-item.svelte';
  import type { ScopeActions, Target } from '$lib/components/types';

  let {
    onclose,
    acts,
  }: {
    onclose: () => void;
    acts: ScopeActions;
  } = $props();

  let menu = $state<ReturnType<typeof ContextMenu> | null>(null);
  let clear = $state<ReturnType<typeof ArmedButton> | null>(null);
  let aimed = $state<Target | null>(null);

  export function open(event: MouseEvent, target: Target) {
    menu?.open(event);
    aimed = target;
  }
</script>

<ContextMenu
  bind:this={menu}
  onclose={() => {
    aimed = null;
    clear?.disarm();
    onclose();
  }}
>
  {#if aimed}
    {@const target = aimed}

    {#if target.busy}
      <MenuItem
        label="Stop"
        Icon={Square}
        act={() => acts.onstop(target.scopes)}
      />
    {:else}
      <MenuItem
        label={TRANSLATE}
        Icon={Languages}
        off={target.excluded || halted()}
        act={() => acts.ontranslate(target.scopes)}
      />
    {/if}
    <MenuItem
      label={APPLY}
      Icon={Upload}
      off={target.excluded || halted()}
      act={() => acts.onexport(target.scopes)}
    />
    <MenuItem
      label={RESTORE}
      Icon={Undo2}
      off={halted() || !target.applied}
      act={() => acts.onrevert(target.scopes)}
    />

    {#if target.excluded}
      <MenuItem
        label="Include in translation"
        Icon={Eye}
        off={halted()}
        act={() => acts.ontoggle(target.scopes, false)}
      />
    {:else}
      <MenuItem
        label="Exclude from translation"
        Icon={EyeOff}
        off={halted()}
        act={() => acts.ontoggle(target.scopes, true)}
      />
    {/if}

    <div class="h-px bg-line"></div>

    <ArmedButton
      bind:this={clear}
      label="Clear translations"
      armedLabel={CLEAR_ARMED}
      Icon={Trash2}
      warning={CLEAR_WARNING}
      disabled={!target.translated || halted()}
      onfire={() => acts.onclear(target.scopes)}
    />
  {/if}
</ContextMenu>
