// The Player: one Spotify.Player, every SDK event from the spec's "Events" list, and the launch sequence.

import { isBridgeError } from '../shared/bridge-error';
import type { ResumePoint, SlopifyBridge, Source } from '../shared/bridge';
import { PlaybackErrorGate, debounce, largestImage, throttle } from './format';
import { clearMediaSession, updateMediaSession } from './media-session';
import type { PlayerState, TrackInfo } from './model';
import { sourceForContext } from './model';

const RECONNECT_MS = 30_000;
const ELSEWHERE_POLL_MS = 10_000;
const TICK_MS = 500;
const RESUME_SAVE_MS = 5_000;
const VOLUME_SAVE_MS = 300;
const MESSAGE_MS = 5_000;
// If the resumed track never shows up in a state event the volume must still come back.
const RESUME_TIMEOUT_MS = 15_000;

type Listener = (state: PlayerState) => void;

function trackInfo(track: Spotify.Track): TrackInfo {
  return {
    id: track.id,
    uri: track.uri,
    name: track.name,
    artists: track.artists.map((a) => ({ name: a.name, uri: a.uri })),
    album: track.album.name,
    imageUrl: largestImage(track.album.images)?.url ?? null,
  };
}

export class Player {
  readonly state: PlayerState = {
    // "Reconnecting" is reserved for not_ready; before the first ready the Dropdown shows the empty state.
    connected: true,
    track: null,
    paused: true,
    positionMs: 0,
    durationMs: 0,
    volume: 0.5,
    source: null,
    elsewhere: null,
    message: null,
  };

  private sdk: Spotify.Player | null = null;
  private readonly listeners = new Set<Listener>();
  private hadReady = false;
  private authRetried = false;
  private reconnectTimer: ReturnType<typeof setInterval> | null = null;
  private elsewhereTimer: ReturnType<typeof setInterval> | null = null;
  private messageTimer: ReturnType<typeof setTimeout> | null = null;
  private resumeTimer: ReturnType<typeof setTimeout> | null = null;
  private tickTimer: ReturnType<typeof setInterval> | null = null;
  private positionAt = 0;
  private basePosition = 0;
  private pendingResume: { trackUri: string | null; volume: number } | null = null;
  private readonly errorGate = new PlaybackErrorGate();
  private knownSources: readonly Source[] = [];
  private userId: string | null = null;

  /** Set when the SDK fails twice on init or auth. Only main can publish an `error` Sign-in State, so the renderer keeps this locally and renders that layout itself. */
  fatal: string | null = null;

  private readonly saveResumeThrottled = throttle((p: ResumePoint) => this.bridge.saveResumePoint(p), RESUME_SAVE_MS);
  private readonly saveVolumeDebounced = debounce((v: number) => this.bridge.saveVolume(v), VOLUME_SAVE_MS);

  constructor(private readonly bridge: SlopifyBridge) {}

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    listener(this.state);
    return () => this.listeners.delete(listener);
  }

  setKnownSources(sources: readonly Source[], userId: string | null): void {
    this.knownSources = sources;
    this.userId = userId;
  }

  /** Builds and connects the SDK player once the SDK script has loaded and auth is signed-in. */
  async start(): Promise<void> {
    if (this.sdk) return;
    this.state.volume = await this.bridge.getVolume();
    const sdk = new window.Spotify.Player({
      name: 'slopify',
      getOAuthToken: (cb) => {
        this.bridge.getAccessToken().then(cb, (error) => console.error('getAccessToken failed', error));
      },
      volume: this.state.volume,
    });
    this.sdk = sdk;

    sdk.addListener('ready', ({ device_id }) => this.onReady(device_id));
    sdk.addListener('not_ready', () => this.onNotReady());
    sdk.addListener('player_state_changed', (s) => this.onState(s));
    sdk.addListener('initialization_error', (e) => this.onAuthError('initialization_error', e));
    sdk.addListener('authentication_error', (e) => this.onAuthError('authentication_error', e));
    sdk.addListener('account_error', (e) => {
      console.error('account_error', e.message);
      this.bridge.reportNotPremium();
    });
    sdk.addListener('playback_error', (e) => this.onPlaybackError(e));
    sdk.addListener('autoplay_failed', () => console.error('autoplay_failed'));

    this.tickTimer = setInterval(() => this.tick(), TICK_MS);
    this.emit();
    await sdk.connect();
  }

  // Transport

  togglePlay(): void {
    if (this.state.elsewhere !== null) {
      this.bridge.transferHere().catch((error) => this.showError(error));
      return;
    }
    if (!this.state.track || !this.state.connected) return;
    this.sdk?.togglePlay().catch((error) => console.error('togglePlay', error));
  }

  play(): void {
    if (this.state.elsewhere !== null) {
      this.togglePlay();
      return;
    }
    this.sdk?.resume().catch((error) => console.error('resume', error));
  }

  pause(): void {
    this.sdk?.pause().catch((error) => console.error('pause', error));
  }

  next(): void {
    if (!this.state.track || !this.state.connected || this.state.elsewhere !== null) return;
    this.sdk?.nextTrack().catch((error) => console.error('nextTrack', error));
  }

  previous(): void {
    this.sdk?.previousTrack().catch((error) => console.error('previousTrack', error));
  }

  setVolume(volume: number): void {
    const v = Math.min(1, Math.max(0, volume));
    this.state.volume = v;
    this.sdk?.setVolume(v).catch((error) => console.error('setVolume', error));
    this.saveVolumeDebounced(v);
    this.emit();
  }

  async startSource(source: Source): Promise<void> {
    this.state.source = source;
    this.emit();
    try {
      await this.bridge.startSource(source);
    } catch (error) {
      this.showError(error);
      throw error;
    }
  }

  // SDK events

  /** A fresh sign-in after `expired` gets the launch sequence again once the SDK reconnects. */
  onSignedIn(): void {
    this.fatal = null;
    this.authRetried = false;
    if (!this.state.connected) this.hadReady = false;
    this.emit();
  }

  private onReady(deviceId: string): void {
    this.bridge.reportDevice(deviceId);
    this.stopReconnect();
    this.state.connected = true;
    this.emit();
    if (!this.hadReady) {
      this.hadReady = true;
      this.launch().catch((error) => console.error('launch', error));
    }
  }

  private onNotReady(): void {
    this.bridge.reportDevice(null);
    this.state.connected = false;
    this.emit();
    if (!this.reconnectTimer) {
      this.reconnectTimer = setInterval(() => {
        this.sdk?.connect().catch((error) => console.error('reconnect', error));
      }, RECONNECT_MS);
    }
  }

  private onState(s: Spotify.PlaybackState | null): void {
    if (!s) {
      this.onElsewhere();
      return;
    }
    this.stopElsewherePoll();
    this.state.elsewhere = null;

    const track = trackInfo(s.track_window.current_track);
    const trackChanged = this.state.track?.uri !== track.uri;
    this.state.track = track;
    this.state.paused = s.paused;
    this.state.durationMs = s.duration;
    this.basePosition = s.position;
    this.positionAt = Date.now();
    this.state.positionMs = s.position;

    const contextSource = sourceForContext(s.context.uri, this.knownSources, this.userId);
    if (contextSource) this.state.source = contextSource;

    updateMediaSession(s);
    this.saveResume(trackChanged || s.paused);
    this.finishResume(track.uri);
    this.emit();
  }

  private onElsewhere(): void {
    if (this.elsewhereTimer) return;
    const poll = () => {
      this.bridge
        .getPlayingElsewhere()
        .then((info) => {
          this.state.elsewhere = info?.deviceName ?? 'another device';
          this.state.paused = true;
          if (info?.track) {
            const idFromUri = info.track.uri.split(':').pop() ?? null;
            this.state.track = { id: idFromUri, ...info.track };
            this.state.positionMs = 0;
          }
          this.emit();
        })
        .catch((error) => console.error('getPlayingElsewhere', error));
    };
    this.state.elsewhere = 'another device';
    this.state.paused = true;
    clearMediaSession();
    this.emit();
    poll();
    this.elsewhereTimer = setInterval(poll, ELSEWHERE_POLL_MS);
  }

  private onAuthError(event: string, error: Spotify.Error): void {
    console.error(event, error.message);
    if (this.authRetried) {
      this.fatal = error.message || 'Spotify rejected the sign-in.';
      this.emit();
      return;
    }
    this.authRetried = true;
    this.bridge
      .getAccessToken()
      .then(() => this.sdk?.connect())
      .catch((e) => {
        console.error('retry after auth error failed', e);
        this.fatal = error.message || 'Spotify rejected the sign-in.';
        this.emit();
      });
  }

  private onPlaybackError(error: Spotify.Error): void {
    console.error('playback_error', error.message);
    const sticky = this.errorGate.record();
    this.showMessage(error.message, sticky);
  }

  // Launch sequence (spec, "Launch sequence")

  private async launch(): Promise<void> {
    const resume = await this.bridge.getResumePoint();
    if (!resume || !this.sdk) return;
    this.state.source = resume.source;
    this.pendingResume = { trackUri: resume.trackUri, volume: this.state.volume };
    this.emit();
    await this.sdk.setVolume(0);
    try {
      await this.bridge.startSource(resume.source, { trackUri: resume.trackUri, positionMs: resume.positionMs });
      this.resumeTimer = setTimeout(() => this.restoreVolume(), RESUME_TIMEOUT_MS);
    } catch (error) {
      this.restoreVolume();
      if (isBridgeError(error) && error.code === 'play-failed' && (error.status === 404 || error.status === 403)) {
        this.bridge.clearResumePoint();
        this.state.source = null;
        this.emit();
        return;
      }
      if (isBridgeError(error) && error.code === 'no-device') {
        this.onNotReady();
        return;
      }
      this.showError(error);
    }
  }

  private finishResume(trackUri: string): void {
    const pending = this.pendingResume;
    if (!pending) return;
    if (pending.trackUri !== null && pending.trackUri !== trackUri) return;
    this.sdk
      ?.pause()
      .catch((error) => console.error('pause after resume', error))
      .finally(() => this.restoreVolume());
  }

  private restoreVolume(): void {
    const pending = this.pendingResume;
    if (!pending) return;
    this.pendingResume = null;
    if (this.resumeTimer) clearTimeout(this.resumeTimer);
    this.resumeTimer = null;
    this.sdk?.setVolume(pending.volume).catch((error) => console.error('restore volume', error));
  }

  // Helpers

  private saveResume(immediate: boolean): void {
    const { source, track, positionMs } = this.state;
    if (!source || !track) return;
    const point: ResumePoint = { source, trackUri: track.uri, positionMs };
    if (immediate) {
      this.saveResumeThrottled.cancel();
      this.bridge.saveResumePoint(point);
    } else {
      this.saveResumeThrottled(point);
    }
  }

  private tick(): void {
    if (this.state.paused || !this.state.track || this.state.elsewhere !== null) return;
    this.state.positionMs = Math.min(this.state.durationMs, this.basePosition + (Date.now() - this.positionAt));
    this.emit();
  }

  private showError(error: unknown): void {
    const message = isBridgeError(error)
      ? error.message || `Spotify refused (${error.code}${error.status ? ` ${error.status}` : ''})`
      : String(error);
    this.showMessage(message, false);
  }

  private showMessage(message: string, sticky: boolean): void {
    if (this.errorGate.isTripped && this.state.message !== null && !sticky) return;
    this.state.message = message;
    if (this.messageTimer) clearTimeout(this.messageTimer);
    this.messageTimer = sticky
      ? null
      : setTimeout(() => {
          this.state.message = null;
          this.messageTimer = null;
          this.emit();
        }, MESSAGE_MS);
    this.emit();
  }

  private stopReconnect(): void {
    if (this.reconnectTimer) clearInterval(this.reconnectTimer);
    this.reconnectTimer = null;
  }

  private stopElsewherePoll(): void {
    if (this.elsewhereTimer) clearInterval(this.elsewhereTimer);
    this.elsewhereTimer = null;
  }

  private emit(): void {
    for (const listener of this.listeners) listener(this.state);
  }
}
