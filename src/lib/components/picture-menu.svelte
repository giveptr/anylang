<script lang="ts">
  import {
    ClipboardPaste,
    Copy,
    Download,
    ImageUp,
    Star,
    X,
  } from '@lucide/svelte';
  import type { Shot } from '$lib/bindings';
  import { marked, swappedTo } from '$lib/pictures.svelte';
  import ContextMenu from '$lib/components/context-menu.svelte';
  import MenuItem from '$lib/components/menu-item.svelte';

  let {
    onexport,
    oncopy,
    onreplace,
    onpaste,
    onclear,
    onmark,
  }: {
    onexport: (shot: Shot) => void;
    oncopy: (shot: Shot) => void;
    onreplace: (shot: Shot) => void;
    onpaste: (shot: Shot) => void;
    onclear: (shot: Shot) => void;
    onmark: (shot: Shot) => void;
  } = $props();

  let menu = $state<ReturnType<typeof ContextMenu> | null>(null);
  let aimed = $state<Shot | null>(null);

  export const shown = () => !!menu?.shown();

  export function open(event: MouseEvent, shot: Shot) {
    menu?.open(event);
    aimed = shot;
  }
</script>

<ContextMenu bind:this={menu} onclose={() => (aimed = null)}>
  {#if aimed}
    {@const shot = aimed}

    <MenuItem
      label="Replace"
      Icon={ImageUp}
      off={!!shot.locked}
      act={() => onreplace(shot)}
    />
    <MenuItem
      label="Paste"
      Icon={ClipboardPaste}
      off={!!shot.locked}
      act={() => onpaste(shot)}
    />

    <div class="h-px bg-line"></div>

    <MenuItem
      label="Export"
      Icon={Download}
      off={!shot.drawable}
      act={() => onexport(shot)}
    />
    <MenuItem
      label="Copy"
      Icon={Copy}
      off={!shot.drawable}
      act={() => oncopy(shot)}
    />
    <MenuItem
      label={marked(shot.key) ? 'Unmark' : 'Mark'}
      Icon={Star}
      act={() => onmark(shot)}
    />

    {#if swappedTo(shot.key)}
      <div class="h-px bg-line"></div>

      <MenuItem label="Clear replacement" Icon={X} act={() => onclear(shot)} />
    {/if}
  {/if}
</ContextMenu>
