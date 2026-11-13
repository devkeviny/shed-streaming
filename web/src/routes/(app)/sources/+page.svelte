<!-- Licensed under the GNU Affero General Public License v3.0 — see LICENSE. -->
<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from '$lib/api';
	import { sources, scenes } from '$lib/stores/pipeline';
	import SourceCard from '../../../components/SourceCard.svelte';
	import { notify } from '$lib/notify';
	import type { SourceKind } from '$lib/types';

	let name = $state('');
	let protocol = $state<'rtmp' | 'srt' | 'browser' | 'webrtc' | 'video_capture' | 'audio_input' | 'text' | 'color'>('rtmp');
	let srtPassphrase = $state('');
	let browserUrl = $state('');
	let creating = $state(false);
	let canvas = $state({ width: 1920, height: 1080 });
	// OBS-style local sources (devices on the Muxshed host itself).
	let v4l2Devices = $state<{ id: string; name: string }[]>([]);
	let alsaDevices = $state<{ id: string; name: string }[]>([]);
	let captureDevice = $state('');
	let captureWidth = $state(1280);
	let captureHeight = $state(720);
	let captureFps = $state(30);
	let audioDevice = $state('default');
	let textContent = $state('');
	let textFontSize = $state(72);
	let textFontColor = $state('#ffffff');
	let textBgColor = $state('#000000');
	let textWidth = $state(1280);
	let textHeight = $state(180);
	let colorValue = $state('#000000');
	let colorWidth = $state(1280);
	let colorHeight = $state(720);

	onMount(async () => {
		await refresh();
		scenes.set(await api.listScenes());
		const cfg = await api.getOutputConfig().catch(() => null);
		if (cfg) canvas = { width: cfg.width, height: cfg.height };
		v4l2Devices = await api.listV4l2Devices().catch(() => []);
		alsaDevices = await api.listAlsaDevices().catch(() => []);
		if (v4l2Devices.length > 0) captureDevice = v4l2Devices[0].id;
	});

	async function refresh() {
		sources.set(await api.listSources());
	}

	async function create() {
		if (!name.trim()) return;
		creating = true;
		try {
			let kind: SourceKind;
			if (protocol === 'srt') {
				kind = {
					type: 'srt',
					port: 0,
					passphrase: srtPassphrase.trim() || undefined,
				};
			} else if (protocol === 'browser') {
				if (!browserUrl.trim()) return;
				kind = { type: 'browser', url: browserUrl.trim() };
			} else if (protocol === 'webrtc') {
				kind = { type: 'web_rtc', token: '' };
			} else if (protocol === 'video_capture') {
				if (!captureDevice) return;
				kind = { type: 'video_capture', device: captureDevice, width: captureWidth, height: captureHeight, fps: captureFps };
			} else if (protocol === 'audio_input') {
				kind = { type: 'audio_input', device: audioDevice };
			} else if (protocol === 'text') {
				if (!textContent.trim()) return;
				kind = { type: 'text', text: textContent.trim(), font_size: textFontSize, font_color: textFontColor, bg_color: textBgColor, width: textWidth, height: textHeight };
			} else if (protocol === 'color') {
				kind = { type: 'color', color: colorValue, width: colorWidth, height: colorHeight };
			} else {
				kind = { type: 'rtmp', stream_key: '' };
			}
			await api.createSource(name.trim(), kind);
			name = '';
			srtPassphrase = '';
			browserUrl = '';
			textContent = '';
			await refresh();
			notify.success('Source added');
		} catch (e) {
			notify.error(e);
		} finally {
			creating = false;
		}
	}
</script>

<div class="mx-auto max-w-4xl space-y-6">
	<section class="panel">
		<header class="panel__head">▮ Add Source</header>
		<div class="panel__body">
			<form onsubmit={(e) => { e.preventDefault(); create(); }} class="space-y-3">
				<div class="flex gap-3">
					<div>
						<label class="field-label" for="source-protocol">Type</label>
						<select id="source-protocol" bind:value={protocol} class="select">
							<option value="rtmp">RTMP</option>
							<option value="srt">SRT</option>
							<option value="browser">Browser</option>
							<option value="webrtc">WHIP (WebRTC)</option>
							<option value="video_capture">Video Capture (USB camera)</option>
							<option value="audio_input">Audio Input (microphone)</option>
							<option value="text">Text</option>
							<option value="color">Color Source</option>
						</select>
					</div>
					<div class="flex-1">
						<label class="field-label" for="source-name">Source Name</label>
						<input
							id="source-name"
							bind:value={name}
							placeholder="e.g. OBS Main"
							class="input"
						/>
					</div>
				</div>
				{#if protocol === 'srt'}
					<div>
						<label class="field-label" for="source-passphrase">Passphrase (optional, min 10 chars)</label>
						<input
							id="source-passphrase"
							bind:value={srtPassphrase}
							placeholder="Passphrase"
							class="input"
						/>
					</div>
				{:else if protocol === 'browser'}
					<div>
						<label class="field-label" for="source-url">Page URL</label>
						<input
							id="source-url"
							bind:value={browserUrl}
							placeholder="https://… (overlay, widget, scoreboard, web page)"
							class="input"
						/>
					</div>
				{:else if protocol === 'video_capture'}
					<div class="flex gap-3">
						<div class="flex-1">
							<label class="field-label" for="source-device">Camera</label>
							<select id="source-device" bind:value={captureDevice} class="select">
								{#each v4l2Devices as d}
									<option value={d.id}>{d.name} ({d.id})</option>
								{/each}
								{#if v4l2Devices.length === 0}
									<option value="">No video devices found</option>
								{/if}
							</select>
						</div>
						<div>
							<label class="field-label" for="source-cap-w">Width</label>
							<input id="source-cap-w" type="number" bind:value={captureWidth} min="160" max="3840" step="2" class="input w-24" />
						</div>
						<div>
							<label class="field-label" for="source-cap-h">Height</label>
							<input id="source-cap-h" type="number" bind:value={captureHeight} min="120" max="2160" step="2" class="input w-24" />
						</div>
						<div>
							<label class="field-label" for="source-cap-fps">FPS</label>
							<input id="source-cap-fps" type="number" bind:value={captureFps} min="1" max="60" class="input w-20" />
						</div>
					</div>
					<p class="text-xs text-amber-muted">USB webcam on the Muxshed box (MJPEG). The codec nodes (meson-venc-h264, meson-video-decoder) are not cameras.</p>
				{:else if protocol === 'audio_input'}
					<div>
						<label class="field-label" for="source-audio">Microphone / input</label>
						<select id="source-audio" bind:value={audioDevice} class="select">
							{#each alsaDevices as d}
								<option value={d.id}>{d.name} ({d.id})</option>
							{/each}
						</select>
					</div>
					<p class="text-xs text-amber-muted">Audio-only: it can't be a scene layer. Select it as the program audio source in the mixer.</p>
				{:else if protocol === 'text'}
					<div>
						<label class="field-label" for="source-text">Text</label>
						<input
							id="source-text"
							bind:value={textContent}
							placeholder="e.g. AO VIVO — Rua Principal"
							class="input"
						/>
					</div>
					<div class="flex gap-3">
						<div>
							<label class="field-label" for="source-font-size">Font size</label>
							<input id="source-font-size" type="number" bind:value={textFontSize} min="8" max="400" class="input w-24" />
						</div>
						<div>
							<label class="field-label" for="source-font-color">Text color</label>
							<input id="source-font-color" type="color" bind:value={textFontColor} class="input h-9 w-16 p-1" />
						</div>
						<div>
							<label class="field-label" for="source-bg-color">Background</label>
							<input id="source-bg-color" type="color" bind:value={textBgColor} class="input h-9 w-16 p-1" />
						</div>
						<div>
							<label class="field-label" for="source-text-w">Width</label>
							<input id="source-text-w" type="number" bind:value={textWidth} min="160" max="3840" step="2" class="input w-24" />
						</div>
						<div>
							<label class="field-label" for="source-text-h">Height</label>
							<input id="source-text-h" type="number" bind:value={textHeight} min="40" max="2160" step="2" class="input w-24" />
						</div>
					</div>
				{:else if protocol === 'color'}
					<div class="flex gap-3">
						<div>
							<label class="field-label" for="source-color">Color</label>
							<input id="source-color" type="color" bind:value={colorValue} class="input h-9 w-16 p-1" />
						</div>
						<div>
							<label class="field-label" for="source-color-w">Width</label>
							<input id="source-color-w" type="number" bind:value={colorWidth} min="160" max="3840" step="2" class="input w-24" />
						</div>
						<div>
							<label class="field-label" for="source-color-h">Height</label>
							<input id="source-color-h" type="number" bind:value={colorHeight} min="120" max="2160" step="2" class="input w-24" />
						</div>
					</div>
				{/if}
				<button type="submit" disabled={creating || !name.trim()} class="btn">
					{creating ? 'Adding…' : '+ Add Source'}
				</button>
			</form>
		</div>
	</section>

	<section class="panel">
		<header class="panel__head">▮ Sources</header>
		<div class="panel__body">
			{#if $sources.length === 0}
				<p class="text-xs text-amber-muted">No sources yet. Create one above.</p>
			{:else}
				<div class="grid gap-3 sm:grid-cols-2">
					{#each $sources as source (source.id)}
						<SourceCard {source} scenes={$scenes} {canvas} ondelete={refresh} />
					{/each}
				</div>
			{/if}
		</div>
	</section>
</div>
