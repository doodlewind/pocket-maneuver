// The game's listing on Pocket Studio: listing/listing.json holds the words and how each picture is
// recorded; `bun tools/listing.ts` writes the pictures to the ignored dist/listing/. This repository
// holds no picture of the listing.
//   bun test ./tools/listing.test.ts          (part of `bun run test`)
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const ROOT = resolve(import.meta.dir, "..");
const listing = JSON.parse(readFileSync(resolve(ROOT, "listing/listing.json"), "utf8"));
const NAME = /^[a-z0-9-]+\.(mp4|jpg|webp|png)$/;

test("the words fit what Pocket Studio takes", () => {
  expect(listing.tagline.length).toBeLessThanOrEqual(120);
  expect(listing.description.length).toBeGreaterThanOrEqual(1);
  expect(listing.description.length).toBeLessThanOrEqual(6);
  for (const paragraph of listing.description) expect(paragraph.length, paragraph).toBeLessThanOrEqual(600);
  expect(listing.media.length).toBeGreaterThanOrEqual(2);
  expect(listing.media.length).toBeLessThanOrEqual(12);
  for (const entry of listing.media) expect(entry.caption.length, entry.caption).toBeLessThanOrEqual(140);
});

test("the Japanese words fit the same limits, with a caption for every picture", () => {
  const ja = listing.translations.ja;
  expect(ja.tagline.length).toBeLessThanOrEqual(120);
  expect(ja.description.length).toBeGreaterThanOrEqual(1);
  expect(ja.description.length).toBeLessThanOrEqual(6);
  for (const paragraph of ja.description) expect(paragraph.length, paragraph).toBeLessThanOrEqual(600);
  expect(Object.keys(ja.captions).sort()).toEqual(listing.media.map((entry: { file: string }) => entry.file).sort());
  for (const caption of Object.values(ja.captions) as string[]) expect(caption.length, caption).toBeLessThanOrEqual(140);
});

test("the title screen and the listing say the same line", () => {
  // (the interface's title, the player's bar and the listing: one sentence, in each language the listing has)
  expect(readFileSync(resolve(ROOT, "wgpu/page/main.js"), "utf8")).toContain(`tagline: { en: ${JSON.stringify(listing.tagline)}, ja: ${JSON.stringify(listing.translations.ja.tagline)} }`);
});

test("the first picture is a clip, and every picture has a take", () => {
  expect(listing.media[0].kind).toBe("video");
  const files = new Set<string>();
  // (the share picture is a still like the others)
  const entries: any[] = [...listing.media, { kind: "image", ...listing.card }];
  for (const entry of entries) {
    expect(entry.file, entry.file).toMatch(NAME);
    expect(files.has(entry.file), entry.file).toBe(false);
    files.add(entry.file);
    const take = entry.take;
    if (entry.kind === "video") {
      const named: boolean = entry.file.endsWith(".mp4") && NAME.test(entry.poster) && entry.poster.endsWith(".jpg");
      expect(named, entry.file).toBe(true);
      // 6 to 30 seconds at 60 frames a second, and a poster inside the clip
      const timed: boolean = take.frames >= 360 && take.frames <= 1800 && take.from >= 0 && take.poster >= 0 && take.poster < take.frames;
      expect(timed, entry.file).toBe(true);
    } else if (take.page) {
      const steps: unknown[][] = take.steps;
      const whole = ["vita", "psp", "3ds", "ipod"].includes(take.page) && steps.length > 0 && steps.every((step) => typeof step[0] === "string" && Number.isInteger(step[1]));
      expect(whole, entry.file).toBe(true);
    } else {
      const sized: boolean = Number.isInteger(take.frame) && take.size.length === 2;
      expect(sized, entry.file).toBe(true);
      // landscape, between 4:3 and 2:1
      const shaped: boolean = take.size[0] / take.size[1] >= 4 / 3 && take.size[0] / take.size[1] <= 2;
      expect(shaped, entry.file).toBe(true);
    }
  }
  expect(listing.card.take.size).toEqual([1200, 630]);
  expect(listing.card.file.endsWith(".jpg")).toBe(true);
});

test("the repository holds no picture of the listing", () => {
  const listed = Bun.spawnSync(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], { cwd: ROOT });
  expect(listed.exitCode).toBe(0);
  const files = listed.stdout.toString().split("\0").filter((file) => file && !file.startsWith("vendor/"));
  expect(files.filter((file) => file.startsWith("listing/"))).toEqual(["listing/listing.json"]);
  expect(files.filter((file) => /\.(mp4|mov|webm|gif)$/i.test(file))).toEqual([]);
});
