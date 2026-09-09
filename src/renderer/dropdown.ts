// Renders the Dropdown from the view model into the static markup in index.html.

import type { AuthState } from '../shared/bridge';
import { artistUrl, formatTime, trackUrl } from './format';
import type { PlayerState } from './model';
import { signInView, sourceName, statusOf } from './model';

export interface DropdownModel {
  auth: AuthState;
  fatal: string | null;
  player: PlayerState;
  pickerOpen: boolean;
}

export interface DropdownActions {
  togglePlay(): void;
  next(): void;
  setVolume(volume: number): void;
  openExternal(url: string): void;
  toggleSource(): void;
  signIn(): void;
  quit(): void;
}

export function el<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing #${id}`);
  return node as T;
}

export class Dropdown {
  private readonly player = el('player');
  private readonly now = el('now');
  private readonly signIn = el('sign-in');
  private readonly signInLine = el('sign-in-line');
  private readonly signInButton = el<HTMLButtonElement>('sign-in-button');
  private readonly art = el<HTMLImageElement>('art');
  private readonly artEmpty = el('art-empty');
  private readonly title = el<HTMLAnchorElement>('title');
  private readonly artists = el('artists');
  private readonly progressFill = el('progress-fill');
  private readonly times = el('times');
  private readonly elapsed = el('elapsed');
  private readonly total = el('total');
  private readonly message = el('message');
  private readonly play = el<HTMLButtonElement>('play');
  private readonly playIcon = el('play-icon') as unknown as SVGUseElement;
  private readonly next = el<HTMLButtonElement>('next');
  private readonly volume = el<HTMLInputElement>('volume');
  private readonly sourceRow = el<HTMLButtonElement>('source-row');
  private readonly sourceThumb = el('source-thumb');
  private readonly sourceImg = el<HTMLImageElement>('source-img');
  private readonly sourceName = el('source-name');
  private readonly displayName = el('display-name');
  private readonly quit = el<HTMLButtonElement>('quit');
  private readonly picker = el('picker');

  private lastArt: string | null = null;
  private lastTrackUri: string | null | undefined = undefined;

  constructor(private readonly actions: DropdownActions) {
    this.play.addEventListener('click', () => actions.togglePlay());
    this.next.addEventListener('click', () => actions.next());
    this.volume.addEventListener('input', () => {
      this.paintVolume(Number(this.volume.value));
      actions.setVolume(Number(this.volume.value) / 100);
    });
    this.sourceRow.addEventListener('click', () => actions.toggleSource());
    this.signInButton.addEventListener('click', () => actions.signIn());
    this.quit.addEventListener('click', () => actions.quit());
    this.title.addEventListener('click', (e) => this.follow(e, this.title));
    this.artists.addEventListener('click', (e) => {
      const link = (e.target as HTMLElement).closest('a');
      if (link) this.follow(e, link);
    });
  }

  render(m: DropdownModel): void {
    const signIn = signInView(m.auth, m.fatal);
    this.signIn.hidden = signIn === null;
    this.player.hidden = signIn !== null;
    this.displayName.textContent = m.auth.kind === 'signed-in' ? m.auth.displayName : '';
    if (signIn) {
      this.signInLine.textContent = signIn.line;
      this.signInButton.disabled = signIn.waiting;
      this.signInButton.textContent = signIn.waiting ? 'Waiting for Spotify' : 'Sign in with Spotify';
      return;
    }
    this.renderPlayer(m.player, m.pickerOpen);
  }

  private renderPlayer(s: PlayerState, pickerOpen: boolean): void {
    const status = statusOf(s);
    const { track } = s;

    this.now.hidden = pickerOpen;
    this.picker.hidden = !pickerOpen;
    this.sourceRow.setAttribute('aria-expanded', String(pickerOpen));

    // Artwork
    const artUrl = track?.imageUrl ?? null;
    if (artUrl !== this.lastArt) {
      this.lastArt = artUrl;
      if (artUrl) this.art.src = artUrl;
      else this.art.removeAttribute('src');
    }
    this.art.hidden = !artUrl;
    this.artEmpty.hidden = !!artUrl;

    // Title and artists, only rebuilt when the track changes
    const uri = track?.uri ?? null;
    if (uri !== this.lastTrackUri) {
      this.lastTrackUri = uri;
      this.renderMeta(s);
    }

    // Progress area
    const line = s.message ?? (status === 'elsewhere' ? `Playing on ${s.elsewhere}` : status === 'reconnecting' ? 'Reconnecting' : null);
    this.times.hidden = line !== null;
    this.message.hidden = line === null;
    this.message.textContent = line ?? '';
    this.message.classList.toggle('error', s.message !== null);
    const pct = s.durationMs > 0 ? Math.min(100, (s.positionMs / s.durationMs) * 100) : 0;
    this.progressFill.style.width = `${pct}%`;
    this.elapsed.textContent = formatTime(track ? s.positionMs : 0);
    this.total.textContent = formatTime(track ? s.durationMs : 0);

    // Transport
    const playing = status === 'playing';
    this.playIcon.setAttribute('href', playing ? '#i-pause' : '#i-play');
    this.play.setAttribute('aria-label', playing ? 'Pause' : 'Play');
    this.play.disabled = status === 'empty' || status === 'reconnecting';
    this.next.disabled = status !== 'playing' && status !== 'paused';
    this.volume.disabled = status === 'empty' || status === 'elsewhere' || status === 'reconnecting';
    if (document.activeElement !== this.volume) {
      const v = Math.round(s.volume * 100);
      this.volume.value = String(v);
      this.paintVolume(v);
    }

    // Source row
    const source = s.source;
    this.sourceName.textContent = sourceName(source);
    this.sourceThumb.classList.toggle('liked', source?.kind === 'liked');
    const thumbUrl = source?.kind === 'playlist' ? source.imageUrl : null;
    if (thumbUrl) {
      if (this.sourceImg.src !== thumbUrl) this.sourceImg.src = thumbUrl;
      this.sourceImg.hidden = false;
    } else {
      this.sourceImg.hidden = true;
    }
  }

  private renderMeta(s: PlayerState): void {
    const { track } = s;
    this.artists.replaceChildren();
    if (!track) {
      this.title.textContent = 'Pick something to play';
      this.title.removeAttribute('title');
      this.title.removeAttribute('href');
      return;
    }
    this.title.textContent = track.name;
    this.title.title = track.name;
    const url = trackUrl(track.uri);
    if (url) this.title.href = url;
    else this.title.removeAttribute('href');

    const names = track.artists.map((a) => a.name).join(', ');
    this.artists.title = names;
    track.artists.forEach((artist, i) => {
      if (i > 0) this.artists.append(', ');
      const link = i === 0 ? artistUrl(artist.uri) : null;
      if (link) {
        const a = document.createElement('a');
        a.href = link;
        a.textContent = artist.name;
        this.artists.append(a);
      } else {
        this.artists.append(artist.name);
      }
    });
  }

  private follow(e: Event, link: HTMLAnchorElement): void {
    e.preventDefault();
    const href = link.getAttribute('href');
    if (href && href !== '#') this.actions.openExternal(href);
  }

  private paintVolume(v: number): void {
    this.volume.style.setProperty('--v', `${v}%`);
  }
}
