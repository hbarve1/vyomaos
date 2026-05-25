// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
	site: 'https://vyomaos.org',
	integrations: [
		starlight({
			title: 'VyomaOS',
			description: 'The WASM-First Operating System — Capability-Secure, 18 MB, Boots in Under 5 Seconds',
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/hbarve1/vyomaos' }],
			customCss: ['./src/styles/custom.css'],
			head: [
				{
					tag: 'meta',
					attrs: { name: 'robots', content: 'index, follow' },
				},
				{
					tag: 'meta',
					attrs: { name: 'keywords', content: 'VyomaOS, WebAssembly, WASM, operating system, capability security, Rust, WASI, wasm32-wasip2, Wasmtime, sandbox, embedded OS' },
				},
				{
					tag: 'meta',
					attrs: { property: 'og:image', content: 'https://vyomaos.org/og-image.png' },
				},
				{
					tag: 'meta',
					attrs: { property: 'og:image:width', content: '1200' },
				},
				{
					tag: 'meta',
					attrs: { property: 'og:image:height', content: '630' },
				},
				{
					tag: 'meta',
					attrs: { name: 'twitter:image', content: 'https://vyomaos.org/og-image.png' },
				},
				{
					tag: 'link',
					attrs: { rel: 'apple-touch-icon', sizes: '180x180', href: '/apple-touch-icon.png' },
				},
				{
					tag: 'link',
					attrs: { rel: 'manifest', href: '/site.webmanifest' },
				},
				{
					tag: 'meta',
					attrs: { name: 'theme-color', content: '#7C3AED' },
				},
				{
					tag: 'script',
					attrs: { type: 'application/ld+json' },
					content: JSON.stringify({
						'@context': 'https://schema.org',
						'@type': 'SoftwareApplication',
						name: 'VyomaOS',
						description: 'A WASM-first operating system built on capability-secure WebAssembly. Every app is a sandboxed wasm32-wasip2 binary.',
						applicationCategory: 'OperatingSystem',
						operatingSystem: 'VyomaOS',
						url: 'https://vyomaos.org',
						license: 'https://github.com/hbarve1/vyomaos/blob/main/LICENSE',
						codeRepository: 'https://github.com/hbarve1/vyomaos',
						programmingLanguage: ['Rust', 'WebAssembly'],
						author: {
							'@type': 'Person',
							name: 'Himank Barve',
							url: 'https://github.com/hbarve1',
						},
					}),
				},
			],
			sidebar: [
				{
					label: 'Getting Started',
					items: [
						{ label: 'Introduction', slug: 'guides/introduction' },
						{ label: 'Quick Start', slug: 'guides/quick-start' },
						{ label: 'Creating an App', slug: 'guides/creating-an-app' },
					],
				},
				{
					label: 'Architecture',
					items: [
						{ label: 'System Overview', slug: 'architecture/overview' },
						{ label: 'Security Model', slug: 'architecture/security' },
						{ label: 'Supervisor (PID 1)', slug: 'architecture/supervisor' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'App Manifest', slug: 'reference/manifest' },
						{ label: 'Display Protocol', slug: 'reference/display-protocol' },
						{ label: 'IPC Broker', slug: 'reference/ipc' },
						{ label: 'Build System', slug: 'reference/build-system' },
					],
				},
				{
					label: 'Project',
					items: [
						{ label: 'Presentation', slug: 'project/presentation' },
						{ label: 'Roadmap', slug: 'project/roadmap' },
						{ label: 'Comparison Matrix', slug: 'project/comparison' },
						{ label: 'Contributing', slug: 'project/contributing' },
					],
				},
			],
		}),
	],
});
