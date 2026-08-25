<script lang="ts">
  type Props = {
    label: string;
    value: string;
    secret?: boolean;
    placeholder?: string;
  };

  let {
    label,
    value = $bindable(),
    secret = false,
    placeholder = '',
  }: Props = $props();
</script>

<label class="flex flex-col gap-1.5">
  <span class="text-sm font-medium">{label}</span>
  {#if secret}
    <input class="box-input" type="password" bind:value {placeholder} />
  {:else}
    <input class="box-input" bind:value {placeholder} />
  {/if}
</label>
