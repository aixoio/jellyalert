<script lang="ts">
	import './layout.css';
	import { page } from '$app/state';
	import favicon from '$lib/assets/favicon.svg';

	let { children } = $props();
	const navigation = [
		{ href: '/', label: 'Overview' },
		{ href: '/shows', label: 'Shows' },
		{ href: '/activity', label: 'Activity' },
		{ href: '/settings', label: 'Settings' }
	];
</script>

<svelte:head><link rel="icon" href={favicon} /></svelte:head>
<div class="shell">
	<a class="btn skip-link" href="#main-content">Skip to content</a>
	<header class="app-header">
		<a class="brand" href="/" aria-label="Jelly Alert overview">
			<svg class="brand-mark" viewBox="0 0 40 40" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M8 23v-5a12 12 0 0 1 24 0v5Z"/><path d="M12 23v7q0 5 4 5m4-12v9m8-9v7q0 5-4 5" stroke-linecap="round"/><path d="M15 17h.01M25 17h.01" stroke-width="3" stroke-linecap="round"/></svg>
			<div><span class="card-title">Jelly Alert</span><p>Notification control</p></div>
		</a>
		<nav aria-label="Main navigation">
			<ul class="menu app-menu">
				{#each navigation as item}
					<li><a href={item.href} class:menu-active={page.url.pathname === item.href} aria-current={page.url.pathname === item.href ? 'page' : undefined}>{item.label}</a></li>
				{/each}
			</ul>
		</nav>
	</header>
	<main id="main-content" class="app-content" tabindex="-1">{@render children()}</main>
</div>
