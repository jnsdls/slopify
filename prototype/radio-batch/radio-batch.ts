// PROTOTYPE, throwaway. Answers issue #8: does Radio produce lists you'd actually play?
// Run: pnpm batch "<artist> - <title>" [...more seeds] [--rounds N]

import Anthropic from "@anthropic-ai/sdk";
import { zodOutputFormat } from "@anthropic-ai/sdk/helpers/zod";
import { z } from "zod";
import http from "node:http";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const CLIENT_ID = "a768335a56b648d4a6d11d945d029ce4";
const REDIRECT = "http://127.0.0.1:8888/callback";
const SCOPES = "user-top-read";
const MODEL = "claude-sonnet-5";
const BATCH = 15;
const ASK = 20;
const HERE = path.dirname(fileURLToPath(import.meta.url));

// ---------- args ----------
const argv = process.argv.slice(2);
const roundsIdx = argv.indexOf("--rounds");
const ROUNDS = roundsIdx >= 0 ? Number(argv[roundsIdx + 1]) : 1;
const seeds = argv.filter((a, i) => a !== "--rounds" && i !== roundsIdx + 1);
if (seeds.length === 0) {
  console.error('usage: pnpm batch "<artist> - <title>" [...] [--rounds N]');
  process.exit(1);
}

// ---------- anthropic ----------
function keychain(service: string): string | undefined {
  try {
    return execFileSync("security", ["find-generic-password", "-s", service, "-w"], { stdio: ["ignore", "pipe", "ignore"] })
      .toString()
      .trim();
  } catch {
    return undefined;
  }
}
const apiKey = process.env.ANTHROPIC_API_KEY ?? keychain("anthropic-api-key");
if (!apiKey) {
  console.error("No Anthropic key. Store one: security add-generic-password -a \"$USER\" -s anthropic-api-key -U -T /usr/bin/security -w");
  process.exit(1);
}
const claude = new Anthropic({ apiKey });

// ---------- spotify auth (PKCE, loopback, nothing persisted) ----------
const b64url = (b: Buffer) => b.toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

async function login(): Promise<string> {
  const verifier = b64url(crypto.randomBytes(48));
  const challenge = b64url(crypto.createHash("sha256").update(verifier).digest());
  const state = b64url(crypto.randomBytes(12));
  const url =
    "https://accounts.spotify.com/authorize?" +
    new URLSearchParams({
      client_id: CLIENT_ID,
      response_type: "code",
      redirect_uri: REDIRECT,
      scope: SCOPES,
      code_challenge_method: "S256",
      code_challenge: challenge,
      state,
    });
  const code = await new Promise<string>((resolve, reject) => {
    const srv = http
      .createServer((req, res) => {
        const u = new URL(req.url ?? "/", "http://127.0.0.1:8888");
        if (u.pathname !== "/callback") {
          res.writeHead(404).end();
          return;
        }
        res.end("<p>slopify prototype: you can close this tab.</p>");
        srv.close();
        if (u.searchParams.get("state") !== state) return reject(new Error("state mismatch"));
        if (u.searchParams.get("error")) return reject(new Error(u.searchParams.get("error")!));
        resolve(u.searchParams.get("code")!);
      })
      .listen(8888, "127.0.0.1", () => {
        console.error("Opening browser for Spotify login...");
        execFileSync("open", [url]);
      });
  });
  const r = await fetch("https://accounts.spotify.com/api/token", {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ grant_type: "authorization_code", code, redirect_uri: REDIRECT, client_id: CLIENT_ID, code_verifier: verifier }),
  });
  const j = await r.json();
  if (!r.ok) throw new Error(`token endpoint ${r.status}: ${JSON.stringify(j)}`);
  return j.access_token as string;
}

// ---------- spotify api ----------
type Track = { name: string; artists: { name: string }[]; external_urls: { spotify: string }; uri: string };
let token = "";

async function spotify(pathAndQuery: string): Promise<any> {
  for (;;) {
    const r = await fetch(`https://api.spotify.com/v1${pathAndQuery}`, { headers: { Authorization: `Bearer ${token}` } });
    if (r.status === 429) {
      const wait = Number(r.headers.get("retry-after") ?? "2");
      console.error(`  rate limited, waiting ${wait}s`);
      await new Promise((f) => setTimeout(f, wait * 1000));
      continue;
    }
    if (!r.ok) throw new Error(`${pathAndQuery} -> ${r.status} ${await r.text()}`);
    return r.json();
  }
}

const label = (t: Track) => `${t.artists.map((a) => a.name).join(", ")} - ${t.name}`;

const norm = (s: string) =>
  s
    .toLowerCase()
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/\s*[\(\[].*?[\)\]]/g, "")
    .replace(/\s*-\s*(remaster|remastered|live|single version|radio edit|mono|stereo|deluxe|feat\.?|ft\.?).*$/i, "")
    .replace(/\s+(feat|ft)\.?\s.*$/i, "")
    .replace(/[^a-z0-9]+/g, " ")
    .trim();

async function resolve(artist: string, title: string): Promise<Track | undefined> {
  const q = `track:${title} artist:${artist}`;
  const j = await spotify(`/search?type=track&limit=10&q=${encodeURIComponent(q)}`);
  const items: Track[] = j.tracks?.items ?? [];
  const wantArtist = norm(artist);
  const wantTitle = norm(title);
  // exact artist + title first, then artist-only match, then nothing
  return (
    items.find((t) => t.artists.some((a) => norm(a.name) === wantArtist) && norm(t.name) === wantTitle) ??
    items.find((t) => t.artists.some((a) => norm(a.name) === wantArtist))
  );
}

// ---------- claude ----------
const BatchSchema = z.object({
  seed_known: z.boolean().describe("true if you recognise the Seed track and artist"),
  candidates: z.array(z.object({ artist: z.string(), title: z.string() })),
});

const SYSTEM = `You name real, released tracks as artist and title pairs for a personal radio station.
Stay close to the Seed's genre, era and energy. Use the Listener's top artists and tracks as a sketch of taste, not as a list to replay. Never repeat anything in the played list, and never repeat the Seed.
Give titles as they appear on Spotify, without remaster or version suffixes, in priority order, best fit first.`;

type Taste = { artists: string[]; tracks: string[] };

async function generate(seed: Track, taste: Taste, played: string[]) {
  const user = [
    `Seed: ${label(seed)}`,
    "",
    `Listener's top artists: ${taste.artists.join("; ")}`,
    "",
    `Listener's top tracks:\n${taste.tracks.join("\n")}`,
    "",
    played.length ? `Played (do not repeat):\n${played.join("\n")}` : "Played: nothing yet.",
    "",
    `Give ${ASK} candidates.`,
  ].join("\n");
  const res = await claude.messages.parse({
    model: MODEL,
    max_tokens: 4000,
    system: SYSTEM,
    messages: [{ role: "user", content: user }],
    output_config: { effort: "low", format: zodOutputFormat(BatchSchema) },
  });
  if (!res.parsed_output) throw new Error(`no parsed output, stop_reason=${res.stop_reason}`);
  const cost = (res.usage.input_tokens * 2 + res.usage.output_tokens * 10) / 1e6;
  return { ...res.parsed_output, usage: res.usage, cost };
}

// ---------- main ----------
token = await login();
const me = await spotify("/me");
console.error(`Logged in as ${me.display_name ?? me.id}`);

const [topA, topT] = await Promise.all([spotify("/me/top/artists?limit=50&time_range=medium_term"), spotify("/me/top/tracks?limit=50&time_range=medium_term")]);
const taste: Taste = { artists: topA.items.map((a: any) => a.name), tracks: topT.items.map((t: Track) => label(t)) };
console.error(`Taste: ${taste.artists.length} artists, ${taste.tracks.length} tracks. Top 5 artists: ${taste.artists.slice(0, 5).join(", ")}`);

const stamp = new Date().toISOString().slice(0, 16).replace(/[:T]/g, "-");
const summary: string[] = [];

for (const seedArg of seeds) {
  const out: string[] = [];
  const say = (s = "") => {
    console.log(s);
    out.push(s);
  };

  const sj = await spotify(`/search?type=track&limit=1&q=${encodeURIComponent(seedArg)}`);
  const seed: Track | undefined = sj.tracks?.items?.[0];
  if (!seed) {
    say(`## Seed "${seedArg}": not found on Spotify, skipped`);
    continue;
  }
  say(`## Seed: ${label(seed)}`);
  say(`Asked for: "${seedArg}"  ->  ${seed.external_urls.spotify}`);

  const played: string[] = [];
  let totalHits = 0;
  let totalAsked = 0;
  let totalCost = 0;

  for (let round = 1; round <= ROUNDS; round++) {
    say();
    say(`### Batch ${round}`);
    const gen = await generate(seed, taste, played);
    totalCost += gen.cost;
    say(`seed_known=${gen.seed_known}  candidates=${gen.candidates.length}  tokens in/out=${gen.usage.input_tokens}/${gen.usage.output_tokens}  cost=$${gen.cost.toFixed(4)}`);
    say();
    let hits = 0;
    let tried = 0;
    for (const c of gen.candidates) {
      if (hits >= BATCH) break;
      tried++;
      const t = await resolve(c.artist, c.title);
      const asked = `${c.artist} - ${c.title}`;
      if (!t) {
        say(`MISS  ${asked}`);
        continue;
      }
      const exact = norm(t.name) === norm(c.title);
      const repeat = played.includes(label(t)) || t.uri === seed.uri;
      hits++;
      played.push(label(t));
      say(`${repeat ? "DUPE" : exact ? "HIT " : "NEAR"}  ${asked}${exact ? "" : `  ->  ${label(t)}`}`);
    }
    totalHits += hits;
    totalAsked += tried;
    say();
    say(`Batch ${round}: ${hits}/${tried} resolved${hits < BATCH ? ` (short of ${BATCH})` : ""}`);
  }

  say();
  say(`Seed total: ${totalHits}/${totalAsked} resolved over ${ROUNDS} batch(es), $${totalCost.toFixed(4)}`);
  say();
  say("Verdict (fill in): would you play this? what's wrong with it?");
  say();

  const file = path.join(HERE, "runs", `${stamp}-${norm(label(seed)).replace(/\s+/g, "-").slice(0, 40)}.md`);
  fs.writeFileSync(file, out.join("\n") + "\n");
  summary.push(`${label(seed)}: ${totalHits}/${totalAsked}, $${totalCost.toFixed(4)}, ${path.relative(process.cwd(), file)}`);
}

console.log("\n=== Summary ===");
for (const s of summary) console.log(s);
