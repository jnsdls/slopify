// Generates the tray template images, the 1024 px app icon and build/icon.icns.
// Run with `node scripts/make-icons.mjs`. Pure Node: PNG encoding via zlib, no image library.
//
// The mark is a rounded square with a play triangle cut out. Deliberately not green and not a
// circle with waves (spec, "Attribution": the icons must not resemble the Spotify mark).
import { deflateSync } from 'node:zlib';
import { mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

// --- PNG writer -------------------------------------------------------------------------------

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
};
const encodePng = (width, height, rgba) => {
  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (width * 4 + 1)] = 0; // filter: none
    rgba.copy(raw, y * (width * 4 + 1) + 1, y * width * 4, (y + 1) * width * 4);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
};

// --- Geometry (all in unit space, 0..1, so every size renders the same mark) ------------------

const inRoundedSquare = (x, y, { x0, y0, x1, y1, r }) => {
  if (x < x0 || x > x1 || y < y0 || y > y1) return false;
  const cx = Math.max(x0 + r, Math.min(x, x1 - r));
  const cy = Math.max(y0 + r, Math.min(y, y1 - r));
  return (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
};
const inTriangle = (x, y, [a, b, c]) => {
  const s = (p, q) => (q[0] - p[0]) * (y - p[1]) - (q[1] - p[1]) * (x - p[0]);
  const d1 = s(a, b), d2 = s(b, c), d3 = s(c, a);
  return !((d1 < 0 || d2 < 0 || d3 < 0) && (d1 > 0 || d2 > 0 || d3 > 0));
};
const inCircle = (x, y, [cx, cy, r]) => (x - cx) ** 2 + (y - cy) ** 2 <= r * r;

// Tray mark, unit space: square inset 2/22, triangle offset right so it looks centred.
const traySquare = { x0: 2 / 22, y0: 2 / 22, x1: 20 / 22, y1: 20 / 22, r: 4.5 / 22 };
const trayTriangle = [[8.5 / 22, 6.5 / 22], [8.5 / 22, 15.5 / 22], [16 / 22, 11 / 22]];
const attentionDot = [18.5 / 22, 3.5 / 22, 3 / 22];
const attentionGap = [18.5 / 22, 3.5 / 22, 4.75 / 22];

const trayShape = (attention) => (x, y) => {
  if (attention && inCircle(x, y, attentionDot)) return true;
  if (attention && inCircle(x, y, attentionGap)) return false;
  return inRoundedSquare(x, y, traySquare) && !inTriangle(x, y, trayTriangle);
};

// App icon: Apple's grid puts the tile at 824/1024 with ~185/1024 corner radius.
const appTile = { x0: 100 / 1024, y0: 100 / 1024, x1: 924 / 1024, y1: 924 / 1024, r: 185 / 1024 };
const appTriangle = [[400 / 1024, 300 / 1024], [400 / 1024, 724 / 1024], [740 / 1024, 512 / 1024]];

// --- Rasteriser: supersampled coverage, then a colour function -------------------------------

const SS = 6;
const render = (size, coverage, colour) => {
  const rgba = Buffer.alloc(size * size * 4);
  for (let py = 0; py < size; py++) {
    for (let px = 0; px < size; px++) {
      let hit = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const x = (px + (sx + 0.5) / SS) / size;
          const y = (py + (sy + 0.5) / SS) / size;
          if (coverage(x, y)) hit++;
        }
      }
      const a = hit / (SS * SS);
      const [r, g, b] = colour(px / size, py / size);
      const i = (py * size + px) * 4;
      rgba[i] = r;
      rgba[i + 1] = g;
      rgba[i + 2] = b;
      rgba[i + 3] = Math.round(a * 255);
    }
  }
  return encodePng(size, size, rgba);
};

const black = () => [0, 0, 0];
// Tile: vertical gradient, deep indigo to violet. Triangle: white.
const appColour = (x, y) => {
  if (inTriangle(x, y, appTriangle)) return [255, 255, 255];
  const t = (y - appTile.y0) / (appTile.y1 - appTile.y0);
  const mix = (a, b) => Math.round(a + (b - a) * t);
  return [mix(70, 44), mix(58, 36), mix(160, 112)];
};
const appShape = (x, y) => inRoundedSquare(x, y, appTile);

// --- Outputs ------------------------------------------------------------------------------------

const resources = join(root, 'resources');
const build = join(root, 'build');
mkdirSync(resources, { recursive: true });
mkdirSync(build, { recursive: true });

writeFileSync(join(resources, 'trayTemplate.png'), render(22, trayShape(false), black));
writeFileSync(join(resources, 'trayTemplate@2x.png'), render(44, trayShape(false), black));
writeFileSync(join(resources, 'trayAttentionTemplate.png'), render(22, trayShape(true), black));
writeFileSync(join(resources, 'trayAttentionTemplate@2x.png'), render(44, trayShape(true), black));

writeFileSync(join(build, 'icon.png'), render(1024, appShape, appColour));

const iconset = join(build, 'icon.iconset');
rmSync(iconset, { recursive: true, force: true });
mkdirSync(iconset);
for (const pt of [16, 32, 128, 256, 512]) {
  writeFileSync(join(iconset, `icon_${pt}x${pt}.png`), render(pt, appShape, appColour));
  writeFileSync(join(iconset, `icon_${pt}x${pt}@2x.png`), render(pt * 2, appShape, appColour));
}
execFileSync('iconutil', ['-c', 'icns', iconset, '-o', join(build, 'icon.icns')], { stdio: 'inherit' });
rmSync(iconset, { recursive: true });
console.log('wrote resources/tray*.png, build/icon.png, build/icon.icns');
