// Bundles the composition without copying public/ (a gigabyte of frames): the
// bundle gets hard links to the files instead, so keep TMPDIR on the same
// volume as this folder (other volumes fall back to copying).
import { bundle } from "@remotion/bundler";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export const TIMEOUT_MS = 2 * 60 * 1000;

const link = (from, to) => {
  fs.mkdirSync(to, { recursive: true });
  for (const entry of fs.readdirSync(from, { withFileTypes: true })) {
    const source = path.join(from, entry.name), target = path.join(to, entry.name);
    if (entry.isDirectory()) link(source, target);
    else {
      try { fs.linkSync(fs.realpathSync(source), target); } catch { fs.copyFileSync(source, target); }
    }
  }
};

export const makeBundle = async () => {
  const empty = fs.mkdtempSync(path.join(os.tmpdir(), "omoba-trailer-public-"));
  const serveUrl = await bundle({ entryPoint: path.resolve("src/index.ts"), publicDir: empty });
  fs.rmSync(empty, { recursive: true, force: true });
  const served = path.join(serveUrl, "public");
  fs.rmSync(served, { recursive: true, force: true });
  link(path.resolve("public"), served);
  return serveUrl;
};

/** Removes a bundle made by `makeBundle` (its links, not the files in public/). */
export const dropBundle = (serveUrl) => fs.rmSync(serveUrl, { recursive: true, force: true });
