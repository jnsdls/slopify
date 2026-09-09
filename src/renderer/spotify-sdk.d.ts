// Minimal ambient types for https://sdk.scdn.co/spotify-player.js, only what slopify touches.

declare namespace Spotify {
  interface Image {
    url: string;
    width: number | null;
    height: number | null;
  }

  interface Artist {
    name: string;
    uri: string;
  }

  interface Album {
    name: string;
    uri: string;
    images: Image[];
  }

  interface Track {
    id: string | null;
    uri: string;
    name: string;
    artists: Artist[];
    album: Album;
    duration_ms: number;
  }

  interface PlaybackState {
    paused: boolean;
    position: number;
    duration: number;
    timestamp: number;
    context: { uri: string | null; metadata: unknown };
    track_window: {
      current_track: Track;
      previous_tracks: Track[];
      next_tracks: Track[];
    };
  }

  interface Error {
    message: string;
  }

  interface WebPlaybackInstance {
    device_id: string;
  }

  interface PlayerInit {
    name: string;
    getOAuthToken(cb: (token: string) => void): void;
    volume?: number;
  }

  class Player {
    constructor(init: PlayerInit);
    connect(): Promise<boolean>;
    disconnect(): void;
    addListener(event: 'ready' | 'not_ready', cb: (instance: WebPlaybackInstance) => void): boolean;
    addListener(event: 'player_state_changed', cb: (state: PlaybackState | null) => void): boolean;
    addListener(
      event: 'initialization_error' | 'authentication_error' | 'account_error' | 'playback_error',
      cb: (error: Error) => void,
    ): boolean;
    addListener(event: 'autoplay_failed', cb: () => void): boolean;
    removeListener(event: string, cb?: (...args: never[]) => void): boolean;
    getCurrentState(): Promise<PlaybackState | null>;
    getVolume(): Promise<number>;
    setVolume(volume: number): Promise<void>;
    pause(): Promise<void>;
    resume(): Promise<void>;
    togglePlay(): Promise<void>;
    seek(positionMs: number): Promise<void>;
    previousTrack(): Promise<void>;
    nextTrack(): Promise<void>;
    activateElement(): Promise<void>;
  }
}

interface Window {
  onSpotifyWebPlaybackSDKReady: () => void;
  Spotify: typeof Spotify;
}
