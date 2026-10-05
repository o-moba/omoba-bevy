import type { Shot } from "./lib";

// Which frames of which clip each scene shows. `from` and `len` are seconds on
// the recorder's clock (frame n of a clip is n / 60 s; ../demo-summary.json has
// each clip's director events); `prepare.py` extracts exactly these ranges.
// `rate` is the playback speed: only the hero-select tour is sped up, gameplay
// runs as recorded.
export const SHOTS = {
  // hero select: the class tour, a finger dragging the class list
  roster: { clip: "phone-wildspark", from: 1.0, len: 12.4, rate: 1.5 } as Shot,
  // mid lane: the fight on the bridge, then rockets and traps under the tower
  wildspark: { clip: "phone-wildspark", from: 33.8, len: 4.8 } as Shot,
  wildsparkPush: { clip: "phone-wildspark", from: 48.3, len: 3.6 } as Shot,
  // top lane: an energy shot, the dash and the melee that follows
  stormfist: { clip: "phone-stormfist", from: 100.6, len: 4.2 } as Shot,
  // bot lane: two enemy heroes
  emberveil: { clip: "phone-emberveil", from: 44.2, len: 4.2 } as Shot,
  // jungle: a camp creature
  warden: { clip: "phone-warden", from: 121.6, len: 4.2 } as Shot,
  // the walk down the bot lane, thumb on the stick
  thumbs: { clip: "phone-emberveil", from: 22.0, len: 4.2 } as Shot,
  // desktop layout
  dawnweaver: { clip: "desktop-dawnweaver", from: 29.5, len: 4.2 } as Shot,
  frostguard: { clip: "desktop-frostguard", from: 34.6, len: 4.2 } as Shot,
  // one second each
  montage: [
    { clip: "phone-wildspark", from: 53.9, len: 1.1 },
    { clip: "phone-stormfist", from: 34.0, len: 1.1 },
    { clip: "phone-warden", from: 122.0, len: 1.1 },
    { clip: "phone-warden", from: 146.0, len: 1.1 },
    { clip: "desktop-dawnweaver", from: 30.0, len: 1.1 },
    { clip: "phone-stormfist", from: 102.2, len: 1.1 },
    { clip: "desktop-frostguard", from: 57.5, len: 1.1 },
    { clip: "desktop-dawnweaver", from: 42.0, len: 1.1 },
  ] as Shot[],
  end: { clip: "phone-wildspark", from: 46.0, len: 4.8 } as Shot,
};
