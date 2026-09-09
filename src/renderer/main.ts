import type { AuthState } from '../shared/bridge';
import { Dropdown } from './dropdown';
import { installMediaHandlers } from './media-session';
import { Picker } from './picker';
import { Player } from './player';

const MAX_HEIGHT = 560;

const bridge = window.slopify;
const player = new Player(bridge);

let auth: AuthState = { kind: 'signed-out', reason: 'first-run' };
let sdkReady = false;
let started = false;

const picker = new Picker({
  listSources: () => bridge.listSources(),
  resolvePastedLink: (text) => bridge.resolvePastedLink(text),
  pick: (source) => player.startSource(source),
  onSources: (sources) => player.setKnownSources(sources, auth.kind === 'signed-in' ? auth.userId : null),
  onOpenChange: () => render(),
});

const dropdown = new Dropdown({
  togglePlay: () => player.togglePlay(),
  next: () => player.next(),
  setVolume: (v) => player.setVolume(v),
  openExternal: (url) => bridge.openExternal(url),
  toggleSource: () => picker.toggle(),
  signIn: () => bridge.signIn().catch((error) => console.error('signIn', error)),
  quit: () => bridge.quit(),
});

function render(): void {
  picker.setCurrent(player.state.source);
  dropdown.render({ auth, fatal: player.fatal, player: player.state, pickerOpen: picker.open });
}

function maybeStart(): void {
  if (started || !sdkReady || auth.kind !== 'signed-in') return;
  started = true;
  picker.preload();
  player.start().catch((error) => console.error('player.start', error));
}

function onAuth(next: AuthState): void {
  auth = next;
  if (auth.kind !== 'signed-in') picker.close();
  else if (started) player.onSignedIn();
  render();
  maybeStart();
}

installMediaHandlers({
  play: () => player.play(),
  pause: () => player.pause(),
  next: () => player.next(),
  previous: () => player.previous(),
});

player.subscribe(() => render());

window.onSpotifyWebPlaybackSDKReady = () => {
  sdkReady = true;
  maybeStart();
};
if (window.Spotify) window.onSpotifyWebPlaybackSDKReady();

bridge.onAuthState(onAuth);
bridge.getAuthState().then(onAuth, (error) => console.error('getAuthState', error));

bridge.onWindowShown((shown) => {
  if (!shown) picker.close();
});

document.addEventListener('keydown', (e) => {
  const inField = e.target instanceof HTMLInputElement && e.target.type === 'text';
  if (e.key === 'Escape') {
    e.preventDefault();
    if (picker.open) picker.close();
    else bridge.hideWindow();
    return;
  }
  if (inField) return;
  if (e.key === ' ') {
    e.preventDefault();
    player.togglePlay();
  } else if (e.key === 'ArrowRight') {
    e.preventDefault();
    player.next();
  }
});

{
  const app = document.getElementById('app') as HTMLElement;
  let lastHeight = 0;
  new ResizeObserver(() => {
    const height = Math.min(MAX_HEIGHT, Math.ceil(app.getBoundingClientRect().height));
    if (height !== lastHeight && height > 0) {
      lastHeight = height;
      bridge.setContentHeight(height);
    }
  }).observe(app);
}
