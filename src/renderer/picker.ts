// The Picker: Liked Songs, the Listener's playlists, and the paste field.

import type { Source } from '../shared/bridge';
import { el } from './dropdown';
import { pasteErrorLine, pickerRows, sameSource, sourceName } from './model';

export interface PickerActions {
  listSources(): Promise<Source[]>;
  resolvePastedLink(text: string): Promise<Source>;
  pick(source: Source): Promise<void>;
  /** Called when the cached list changes so the Player can match context uris. */
  onSources(sources: Source[]): void;
  onOpenChange(open: boolean): void;
}

export class Picker {
  private readonly list = el<HTMLUListElement>('picker-list');
  private readonly paste = el<HTMLInputElement>('paste');
  private readonly pasteError = el('paste-error');
  private sources: Source[] = [];
  private current: Source | null = null;
  private refreshing: Promise<void> | null = null;
  open = false;

  constructor(private readonly actions: PickerActions) {
    this.list.addEventListener('click', (e) => {
      const row = (e.target as HTMLElement).closest<HTMLLIElement>('li[data-index]');
      if (!row) return;
      const source = this.rows()[Number(row.dataset.index)];
      if (source) this.choose(source);
    });
    this.paste.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        this.submitPaste();
      }
    });
    this.paste.addEventListener('input', () => this.showPasteError(null));
  }

  /** Loads the list once at startup so the first open renders without waiting. */
  preload(): void {
    this.refresh();
  }

  setCurrent(source: Source | null): void {
    if (sameSource(source, this.current)) return;
    this.current = source;
    if (this.open) this.renderList();
  }

  toggle(): void {
    this.setOpen(!this.open);
  }

  close(): void {
    this.setOpen(false);
  }

  private setOpen(open: boolean): void {
    if (this.open === open) return;
    this.open = open;
    if (open) {
      this.paste.value = '';
      this.paste.disabled = false;
      this.showPasteError(null);
      this.renderList();
      this.refresh();
    }
    this.actions.onOpenChange(open);
  }

  private refresh(): void {
    if (this.refreshing) return;
    this.refreshing = this.actions
      .listSources()
      .then((sources) => {
        this.sources = sources;
        this.actions.onSources(sources);
        if (this.open) this.renderList();
      })
      .catch((error) => console.error('listSources', error))
      .finally(() => {
        this.refreshing = null;
      });
  }

  private rows(): Source[] {
    return pickerRows(this.sources, this.current);
  }

  private renderList(): void {
    const rows = this.rows();
    this.list.replaceChildren(
      ...rows.map((source, index) => {
        const li = document.createElement('li');
        li.dataset.index = String(index);
        li.setAttribute('role', 'option');
        li.setAttribute('aria-selected', String(sameSource(source, this.current)));

        const check = svgUse('#i-check', 'icon check');
        const thumb = document.createElement('span');
        thumb.className = 'thumb';
        if (source.kind === 'liked') {
          thumb.classList.add('liked');
          thumb.append(svgUse('#i-heart', 'icon'));
        } else if (source.imageUrl) {
          const img = document.createElement('img');
          img.src = source.imageUrl;
          img.alt = '';
          thumb.append(img);
        }

        const name = document.createElement('span');
        name.className = 'ellip';
        name.textContent = sourceName(source);
        name.title = sourceName(source);

        li.append(check, thumb, name);
        return li;
      }),
    );
  }

  private async choose(source: Source): Promise<void> {
    this.close();
    try {
      await this.actions.pick(source);
    } catch (error) {
      console.error('startSource', error);
    }
  }

  private async submitPaste(): Promise<void> {
    const text = this.paste.value.trim();
    if (!text) return;
    this.paste.disabled = true;
    this.showPasteError(null);
    try {
      const source = await this.actions.resolvePastedLink(text);
      await this.choose(source);
    } catch (error) {
      const code = typeof error === 'object' && error !== null ? (error as { code?: string }).code : undefined;
      this.showPasteError(pasteErrorLine(code));
    } finally {
      this.paste.disabled = false;
      if (this.open) this.paste.focus();
    }
  }

  private showPasteError(line: string | null): void {
    this.pasteError.hidden = line === null;
    this.pasteError.textContent = line ?? '';
  }
}

function svgUse(href: string, className: string): SVGSVGElement {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('class', className);
  const use = document.createElementNS(ns, 'use');
  use.setAttribute('href', href);
  svg.append(use);
  return svg;
}
