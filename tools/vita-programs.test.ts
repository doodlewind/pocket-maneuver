// The check `tools/release.ts` runs before it packs the PS Vita's programs: a set is taken only with the record
// of the pass that collected it (`bun tools/maneuver.ts programs`), for this checkout's shader sources and for
// the pack being packaged.
import { expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { coverageFault, shaderSources, type Coverage } from "./vita.ts";

const PACK = "ab".repeat(32);
const PROGRAMS = ["0123456789abcdef", "fedcba9876543210"];

/** A directory as a pass leaves it, with `change` applied to its record and `drop` left out. */
function directory(change: Partial<Coverage> = {}, drop: string[] = []): string {
  const at = mkdtempSync(join(tmpdir(), "vita-programs-"));
  const record: Coverage = {
    schema: 1, at: "2026-10-06T00:00:00.000Z", commit: null, dirty: false, nativeBuild: "0".repeat(32),
    packSha256: PACK, shaderSources: shaderSources(), requested: 3, compiled: 3, programs: PROGRAMS, ...change,
  };
  const files: Record<string, string> = { "coverage.json": JSON.stringify(record), "manifest.txt": PROGRAMS.join("\n") };
  for (const name of PROGRAMS) files[`${name}.gxp`] = "GXP\0 a program's bytes";
  for (const [name, text] of Object.entries(files)) if (!drop.includes(name)) writeFileSync(join(at, name), text);
  return at;
}

function fault(change: Partial<Coverage> = {}, drop: string[] = [], pack = PACK): string | null {
  const at = directory(change, drop);
  try {
    return coverageFault(at, pack);
  } finally {
    rmSync(at, { recursive: true, force: true });
  }
}

test("the set of a pass over this checkout's sources and this pack is taken", () => {
  expect(fault()).toBeNull();
});

test("a set without its coverage record is refused", () => {
  expect(fault({}, ["coverage.json"])).toContain("coverage.json is missing");
});

test("a set compiled from other shader sources is refused", () => {
  expect(fault({ shaderSources: "cd".repeat(32) })).toContain("other shader sources");
});

test("a set compiled for another pack is refused", () => {
  expect(fault({}, [], "ef".repeat(32))).toContain("another pack");
});

test("a pass that did not compile every program it asked for is refused", () => {
  expect(fault({ compiled: 2 })).toContain("compiled 2 of 3");
});

test("a list that is not the record's is refused", () => {
  expect(fault({ programs: [PROGRAMS[0]!] })).toContain("manifest.txt is not the list");
});

test("a set that lacks a listed program is refused", () => {
  expect(fault({}, [`${PROGRAMS[1]}.gxp`])).toContain(`${PROGRAMS[1]}.gxp is missing`);
});
