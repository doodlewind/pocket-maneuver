#!/usr/bin/env bun
// The game's listing on Pocket Studio: the words of listing/listing.json and
// the pictures it names, recorded from the game itself.
//
//   bun tools/listing.ts [--only NAME…]   → dist/listing/: listing.json, the clips and their posters,
//                                           the stills, the share picture
//   bun tools/listing.ts --words          listing.json alone, for the pictures already in dist/listing/
//   bun tools/listing.ts --upload         the same, then `pocket-studio listing dist/listing`, run here,
//                                         where `pocket-studio register` wrote .pocket-studio.json.
//                                         POCKET_STUDIO_CLI names the command when it is not on PATH.
//
// Every picture is drawn by the wgpu renderer (wgpu/), which reads the PS Vita's pack: `from: "browser"`
// in the listing. A take in listing/listing.json says how one is made:
//
//   { "words", "from", "frames", "poster" }   a clip: frames `from` to `from + frames` of the run that
//                                             `words` start (App::control), at 60 a second and the
//                                             PS Vita's 960 × 544; the poster is frame `poster`
//   { "words", "frame", "size" }              a still: frame `frame`, drawn at `size`
//   { "page", "steps" }                       a still with the interface over the scene: the page
//                                             (wgpu/page) as the device `page`, in Chrome, its display
//                                             loop stopped; each step is words and a count of frames
//
// A run is the simulation from its first tick, a sixtieth of a second a frame, flown by the autopilot: a
// take of the same numbers is the same picture. The pack is `bun tools/wgpu.ts cook`'s; ffmpeg encodes.
// No picture goes to Git: dist/ is ignored.

import { $ } from "bun";
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { CHROME, PACK, ROOT, build, serve, shotProgram } from "./wgpu.ts";

const OUT = join(ROOT, "dist/listing");
const WORK = join(ROOT, ".pocket-build/listing");
// The PS Vita's screen: a clip has its pixels.
const [WIDTH, HEIGHT] = [960, 544];
// What Pocket Studio takes (the listing's contract): a clip's length and size, a still's and the share
// picture's size, the entries, the whole directory.
const LIMIT = { clip: 12 << 20, seconds: [6, 30], still: 2 << 20, card: 1 << 20, media: [2, 12], total: 96 << 20, tagline: 120, paragraph: 600, paragraphs: [1, 6], caption: 140 };

type Take = { words?: string; from?: number; frames?: number; poster?: number; frame?: number; size?: [number, number]; page?: string; steps?: [string, number][] };
type Entry = { kind: "video" | "image"; file: string; poster?: string; caption: string; take: Take };
type Source = { tagline: string; description: string[]; media: Entry[]; card: { file: string; take: Take }; translations?: Translations };

/**
 * The listing's words in another language (the contract, version 1.1): `translations.<lang>` may hold a
 * tagline, the description's paragraphs and a caption a picture, each under the English limits. A word
 * it leaves out is shown in English.
 */
type Translations = Record<string, { tagline?: string; description?: string[]; captions?: Record<string, string> }>;
function translationFaults(media: { file: string }[], translations: Translations | undefined): string[] {
  const faults: string[] = [];
  const files = new Set(media.map((entry) => entry.file));
  for (const [lang, words] of Object.entries(translations ?? {})) {
    if (!/^[a-z]{2}$/.test(lang)) faults.push(`translations: "${lang}" is not a language`);
    if (words.tagline !== undefined && (!words.tagline || words.tagline.length > 120)) faults.push(`translations.${lang}: the tagline is one sentence of at most 120 characters`);
    if (words.description !== undefined && (words.description.length < 1 || words.description.length > 6 || words.description.some((p) => !p || p.length > 600))) faults.push(`translations.${lang}: the description is one to six paragraphs of at most 600 characters`);
    for (const [file, caption] of Object.entries(words.captions ?? {})) {
      if (!files.has(file)) faults.push(`translations.${lang}: a caption for ${file}, which the listing does not name`);
      if (!caption || caption.length > 140) faults.push(`translations.${lang}: the caption of ${file} has at most 140 characters`);
    }
  }
  return faults;
}

const rest = process.argv.slice(2);
const only = rest.flatMap((flag, i) => (flag === "--only" && rest[i + 1] ? [rest[i + 1]!] : []));
// (--words: the words change and the pictures stay; nothing is recorded)
const words = rest.includes("--words");
const wanted = (file: string) => !words && (only.length === 0 || only.some((name) => file.startsWith(name)));
const source = JSON.parse(readFileSync(join(ROOT, "listing/listing.json"), "utf8")) as Source;
if (!words && !existsSync(PACK)) throw new Error(`${PACK} is missing: bun tools/wgpu.ts cook`);
if (!Bun.which("ffmpeg")) throw new Error("ffmpeg is not on PATH");
mkdirSync(OUT, { recursive: true });
mkdirSync(WORK, { recursive: true });
const program = words ? "" : await shotProgram();

/** A PNG as a JPEG of the same pixels. */
async function jpeg(png: string, out: string, quality = 2) {
  await $`ffmpeg -v error -y -i ${png} -q:v ${quality} -pix_fmt yuvj420p ${out}`;
}

/** One frame of a run on this machine's GPU, as a JPEG. */
async function still(out: string, take: Take, size: [number, number], quality = 2) {
  const png = join(WORK, "frame.png");
  await $`${program} --pack ${PACK} --out ${png} --frames ${take.frame! + 1} --size ${size.join("x")} --words ${take.words ?? ""}`.quiet();
  await jpeg(png, out, quality);
}

/** Frames of a run as an H.264 clip with no sound, its index at the front, and the poster's frame. */
async function clip(entry: Entry) {
  const { words = "", from = 0, frames = 600, poster = 0 } = entry.take;
  const out = join(OUT, entry.file);
  // (the encoder reads the frames as the renderer writes them; a clip over the limit is encoded again, coarser)
  for (const crf of [20, 22, 24, 26, 28]) {
    await $`${program} --pack ${PACK} --film - --from ${from} --frames ${from + frames} --words ${words} | ffmpeg -v error -y -f rawvideo -pix_fmt rgba -s ${WIDTH}x${HEIGHT} -r 60 -i - -an -c:v libx264 -profile:v high -preset slow -crf ${crf} -pix_fmt yuv420p -movflags +faststart ${out}`.quiet();
    if (statSync(out).size <= LIMIT.clip) break;
  }
  await still(join(OUT, entry.poster!), { words, frame: from + poster }, [WIDTH, HEIGHT]);
}

/** Stills of the page with the interface over the scene: the device's own screen, a pixel to a pixel. */
async function pages(entries: Entry[]) {
  if (entries.length === 0) return;
  await build();
  const { chromium } = await import("playwright-core");
  const server = serve(0);
  const browser = await chromium.launch({ channel: "chrome", headless: true, args: CHROME });
  try {
    for (const entry of entries) {
      const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
      await page.goto(`http://127.0.0.1:${server.port}/?device=${entry.take.page}&sound=off`);
      await page.waitForFunction("window.pocketManeuver && ((pocketManeuver.firstWorld && pocketManeuver.interface?.()) || pocketManeuver.failure)", undefined, { timeout: 90_000 });
      const failure = (await page.evaluate("pocketManeuver.failure")) as string;
      if (failure) throw new Error(`${entry.file}: ${failure}`);
      let shot = "";
      for (const [words, count] of entry.take.steps ?? []) shot = ((await page.evaluate(`pocketManeuver.take(${JSON.stringify(words)}, ${count})`)) as { upper: string }).upper;
      const png = join(WORK, "page.png");
      writeFileSync(png, Buffer.from(shot.slice(shot.indexOf(",") + 1), "base64"));
      await jpeg(png, join(OUT, entry.file));
      await page.close();
    }
  } finally {
    await browser.close();
    server.stop(true);
  }
}

for (const entry of source.media) {
  if (!wanted(entry.file)) continue;
  if (entry.kind === "video") await clip(entry);
  else if (!entry.take.page) await still(join(OUT, entry.file), entry.take, entry.take.size ?? [WIDTH, HEIGHT]);
  console.log(`listing: ${entry.file}`);
}
if (!words) await pages(source.media.filter((entry) => entry.take.page && wanted(entry.file)));
if (wanted(source.card.file)) {
  // (the share picture: coarser steps until it is under the limit)
  for (const quality of [2, 3, 4, 6]) {
    await still(join(OUT, source.card.file), source.card.take, source.card.take.size ?? [1200, 630], quality);
    if (statSync(join(OUT, source.card.file)).size <= LIMIT.card) break;
  }
}

// ---------------------------------------------------------------- the directory, as the contract states it

/** What ffprobe says of a file's picture. */
async function probe(file: string) {
  const stream = JSON.parse(await $`ffprobe -v error -select_streams v:0 -show_entries stream=width,height,codec_name,profile,pix_fmt,nb_frames,r_frame_rate:format=duration -of json ${join(OUT, file)}`.text());
  const audio = JSON.parse(await $`ffprobe -v error -select_streams a -show_entries stream=index -of json ${join(OUT, file)}`.text());
  return { ...stream.streams[0], duration: Number(stream.format.duration), audio: (audio.streams ?? []).length, bytes: statSync(join(OUT, file)).size } as { width: number; height: number; codec_name: string; profile: string; pix_fmt: string; r_frame_rate: string; duration: number; audio: number; bytes: number };
}

const refused: string[] = [];
const refuse = (what: string, ok: boolean) => ok || refused.push(what);
const named = (file: string) => /^[a-z0-9-]+\.(mp4|jpg|webp|png)$/.test(file);
const media = [];
let total = 0;
for (const entry of source.media) {
  refuse(`${entry.file}: its name`, named(entry.file));
  refuse(`${entry.file}: a caption of ${entry.caption.length} characters`, entry.caption.length <= LIMIT.caption);
  const seen = await probe(entry.file);
  total += seen.bytes;
  refuse(`${entry.file}: ${seen.width} by ${seen.height}`, seen.width / seen.height >= 4 / 3 && seen.width / seen.height <= 2);
  if (entry.kind === "video") {
    const poster = await probe(entry.poster!);
    total += poster.bytes;
    refuse(`${entry.file}: ${seen.codec_name} ${seen.profile} ${seen.pix_fmt}, ${seen.audio} sound track(s), ${seen.r_frame_rate} frames a second`, seen.codec_name === "h264" && ["High", "Main"].includes(seen.profile) && seen.pix_fmt === "yuv420p" && seen.audio === 0 && ["60/1", "30/1"].includes(seen.r_frame_rate));
    refuse(`${entry.file}: ${seen.duration} s`, seen.duration >= LIMIT.seconds[0]! && seen.duration <= LIMIT.seconds[1]!);
    refuse(`${entry.file}: ${seen.bytes} bytes`, seen.bytes <= LIMIT.clip);
    refuse(`${entry.poster}: its name, or not the clip's size`, named(entry.poster!) && entry.poster!.endsWith(".jpg") && poster.width === seen.width && poster.height === seen.height);
    media.push({ kind: "video", file: entry.file, poster: entry.poster, width: seen.width, height: seen.height, seconds: Math.round(seen.duration), from: "browser", caption: entry.caption });
  } else {
    refuse(`${entry.file}: ${seen.bytes} bytes`, seen.bytes <= LIMIT.still);
    media.push({ kind: "image", file: entry.file, width: seen.width, height: seen.height, from: "browser", caption: entry.caption });
  }
}
const card = await probe(source.card.file);
total += card.bytes;
refuse(`${source.card.file}: ${card.width} by ${card.height}, ${card.bytes} bytes`, card.width === 1200 && card.height === 630 && card.bytes <= LIMIT.card && source.card.file.endsWith(".jpg"));
refuse(`${source.media.length} pictures`, source.media.length >= LIMIT.media[0]! && source.media.length <= LIMIT.media[1]!);
refuse("the first picture is the lead: a clip", source.media[0]?.kind === "video");
refuse(`a tagline of ${source.tagline.length} characters`, source.tagline.length <= LIMIT.tagline);
refuse(`${source.description.length} paragraphs`, source.description.length >= LIMIT.paragraphs[0]! && source.description.length <= LIMIT.paragraphs[1]! && source.description.every((p) => p.length <= LIMIT.paragraph));
refuse(`${total} bytes in all`, total <= LIMIT.total);
for (const fault of translationFaults(source.media, source.translations)) refuse(fault, false);
writeFileSync(join(OUT, "listing.json"), JSON.stringify({ tagline: source.tagline, description: source.description, media, card: source.card.file, ...(source.translations ? { translations: source.translations } : {}) }, null, 2) + "\n");
console.table(media.map((m) => ({ file: m.file, size: `${m.width}x${m.height}`, seconds: "seconds" in m ? m.seconds : "", kB: Math.round(statSync(join(OUT, m.file)).size / 1000) })));
console.log(`listing: ${OUT} (${media.length} pictures, ${(total / 1e6).toFixed(1)} MB with ${source.card.file})`);
if (refused.length) throw new Error(`Pocket Studio would refuse the listing: ${refused.join("; ")}`);
rmSync(WORK, { recursive: true, force: true });

if (rest.includes("--upload")) {
  const link = join(ROOT, ".pocket-studio.json");
  if (!existsSync(link)) throw new Error(`${link} is missing: run \`pocket-studio register --title "Pocket Maneuver"\` here, or copy the link file of the registered game`);
  const cli = process.env.POCKET_STUDIO_CLI?.trim().split(/\s+/) ?? (Bun.which("pocket-studio") ? ["pocket-studio"] : null);
  if (!cli) throw new Error("no pocket-studio on PATH: install it from the Studio, or set POCKET_STUDIO_CLI to its command");
  const command = [...cli, "listing", OUT];
  console.log(`$ ${command.join(" ")}`);
  const code = await Bun.spawn(command, { cwd: ROOT, stdin: "ignore", stdout: "inherit", stderr: "inherit" }).exited;
  if (code !== 0) throw new Error(`pocket-studio listing exited ${code}`);
}
