// Review stills: bundle once, render the given seconds as half-size JPEGs.
//   node stills.mjs 1.8 5 9.2 ...        -> out/stills/s-0001.80.jpg ...
import { renderStill, selectComposition } from "@remotion/renderer";
import fs from "node:fs";
import path from "node:path";
import { TIMEOUT_MS, dropBundle, makeBundle } from "./bundle.mjs";

const seconds = process.argv.slice(2).map(Number);
const scale = Number(process.env.STILL_SCALE ?? 0.5);
const browserExecutable = process.env.BROWSER || undefined;
const out = path.resolve("out/stills");
fs.mkdirSync(out, { recursive: true });
const serveUrl = await makeBundle();
const composition = await selectComposition({ serveUrl, id: "TrailerV2", browserExecutable, timeoutInMilliseconds: TIMEOUT_MS });
for (const s of seconds) {
  const frame = Math.min(composition.durationInFrames - 1, Math.round(s * composition.fps));
  const output = path.join(out, `s-${s.toFixed(2).padStart(7, "0")}.jpg`);
  await renderStill({ composition, serveUrl, output, frame, imageFormat: "jpeg", jpegQuality: 88, scale, browserExecutable, timeoutInMilliseconds: TIMEOUT_MS });
  console.log(output);
}
dropBundle(serveUrl);
