<script lang="ts">
    let { id, fullWidth = false }: { id: number; fullWidth?: boolean } = $props();
    let failed = $state(false);
</script>

<figure class={fullWidth ? "h-40 w-full overflow-hidden bg-base-200 sm:h-48" : "aspect-2/3 w-20 shrink-0 overflow-hidden rounded-box bg-base-200"}>
    {#if failed}
        <div class="grid size-full place-content-center" aria-label="Artwork unavailable">
            <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="2"/><path d="m9 10 5 3-5 3Z"/></svg>
        </div>
    {:else}
        <img class={fullWidth ? "size-full object-contain" : "size-full object-cover"} src={`/api/shows/${id}/poster`} alt="" width="80" height="120" loading="lazy" decoding="async" onerror={() => failed = true} />
    {/if}
</figure>
