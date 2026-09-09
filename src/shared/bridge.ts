// The one object the preload exposes as window.slopify. Shape is fixed by docs/spec/v1.md.

export type Source =
  | { kind: 'playlist'; id: string; uri: string; name: string; imageUrl: string | null; pasted: boolean }
  | { kind: 'liked' };

export type ResumePoint = { source: Source; trackUri: string | null; positionMs: number };

export type SignedOutReason = 'first-run' | 'expired' | 'not-premium' | 'error';

export type AuthState =
  | { kind: 'signed-out'; reason: SignedOutReason; detail?: string }
  | { kind: 'signing-in' }
  | { kind: 'signed-in'; displayName: string; userId: string };

export type PastedLinkErrorCode = 'bad-link' | 'not-found' | 'forbidden';

/** Errors crossing the bridge carry a `code`; the renderer switches on it. */
export type BridgeErrorCode = PastedLinkErrorCode | 'no-device' | 'play-failed';

export interface BridgeError {
  code: BridgeErrorCode;
  status?: number;
  message?: string;
}

/** What another Connect device is playing, from GET /me/player. Feeds the "Playing on" state. */
export interface PlayingElsewhere {
  deviceName: string;
  track?: {
    uri: string;
    name: string;
    artists: { name: string; uri: string }[];
    album: string;
    imageUrl: string | null;
  };
}

export interface SlopifyBridge {
  // auth
  getAuthState(): Promise<AuthState>;
  onAuthState(cb: (s: AuthState) => void): () => void;
  signIn(): Promise<void>;
  getAccessToken(): Promise<string>;
  /** The SDK's account_error: the main process publishes signed-out / not-premium. */
  reportNotPremium(): void;

  // sources
  listSources(): Promise<Source[]>;
  resolvePastedLink(text: string): Promise<Source>;

  // playback commands that go through the Web API
  startSource(source: Source, resume?: { trackUri: string | null; positionMs: number }): Promise<void>;
  transferHere(): Promise<void>;
  getPlayingElsewhere(): Promise<PlayingElsewhere | null>;

  // state the main process persists
  getResumePoint(): Promise<ResumePoint | null>;
  saveResumePoint(p: ResumePoint): void;
  clearResumePoint(): void;
  getVolume(): Promise<number>;
  saveVolume(v: number): void;

  // player identity
  reportDevice(deviceId: string | null): void;

  // shell
  openExternal(url: string): void;
  quit(): void;
  hideWindow(): void;
  /** Fires when the Dropdown is shown or hidden; the renderer resets the Picker on hide. */
  onWindowShown(cb: (shown: boolean) => void): () => void;
  /** The renderer reports its content height; main sizes the window to it, capped at 560. */
  setContentHeight(px: number): void;
}

/** IPC channel names, one per bridge method, shared by preload and main. */
export const ipc = {
  getAuthState: 'auth:get',
  authState: 'auth:state',
  signIn: 'auth:sign-in',
  getAccessToken: 'auth:token',
  reportNotPremium: 'auth:not-premium',
  listSources: 'sources:list',
  resolvePastedLink: 'sources:resolve',
  startSource: 'playback:start',
  transferHere: 'playback:transfer',
  getPlayingElsewhere: 'playback:elsewhere',
  getResumePoint: 'state:resume:get',
  saveResumePoint: 'state:resume:save',
  clearResumePoint: 'state:resume:clear',
  getVolume: 'state:volume:get',
  saveVolume: 'state:volume:save',
  reportDevice: 'player:device',
  openExternal: 'shell:open',
  quit: 'shell:quit',
  hideWindow: 'shell:hide',
  windowShown: 'shell:shown',
  resize: 'window:resize',
} as const;

declare global {
  interface Window {
    slopify: SlopifyBridge;
  }
}
