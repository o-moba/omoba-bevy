// Renders the trailer: 1920x1080, 60 fps, H.264 (BT.709) + AAC.
//   node render.mjs [out/omoba-trailer-v2.mp4]
// BROWSER=<chrome-headless-shell> uses a specific browser; CONCURRENCY sets the tab count;
// FRAMES=1920-2159 renders only that frame range (a check of one scene).
import { renderMedia, selectComposition } from "@remotion/renderer";
import path from "node:path";
import { TIMEOUT_MS, dropBundle, makeBundle } from "./bundle.mjs";

const outputLocation = path.resolve(process.argv[2] ?? "out/omoba-trailer-v2.mp4");
const browserExecutable = process.env.BROWSER || undefined;
const serveUrl = await makeBundle();
const composition = await selectComposition({ serveUrl, id: "TrailerV2", browserExecutable, timeoutInMilliseconds: TIMEOUT_MS });
let last = -1;
await renderMedia({
  composition, serveUrl, outputLocation, browserExecutable, timeoutInMilliseconds: TIMEOUT_MS,
  codec: "h264", crf: 16, pixelFormat: "yuv420p", colorSpace: "bt709", audioBitrate: "192k",
  concurrency: Number(process.env.CONCURRENCY ?? 6),
  frameRange: process.env.FRAMES ? process.env.FRAMES.split("-").map(Number) : null,
  onProgress: ({ progress }) => {
    const percent = Math.floor(progress * 20) * 5;
    if (percent !== last) { last = percent; console.log(`${percent}%`); }
  },
});
dropBundle(serveUrl);
console.log(outputLocation);
