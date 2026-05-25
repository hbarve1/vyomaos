// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
	integrations: [
		starlight({
			title: 'VyomaOS',
			description: 'The WASM-First Operating System — Capability-Secure, 18 MB, Boots in Under 5 Seconds',
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/hbarve1/vyomaos' }],
			customCss: ['./src/styles/custom.css'],
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
