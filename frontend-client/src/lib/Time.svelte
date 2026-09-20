<script lang="ts">
	let { value, empty = 'Not yet' }: { value: number | null; empty?: string } = $props();
</script>
{#if value}<time datetime={new Date(value * 1000).toISOString()}>{new Date(value * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })}</time>{:else}{empty}{/if}
