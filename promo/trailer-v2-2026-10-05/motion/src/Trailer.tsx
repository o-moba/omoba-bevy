import React from "react";
import {
  AbsoluteFill, Audio, Sequence, interpolate, staticFile,
  F, FPS, W, H, useT, INK, DEEP, TEAL, EMERALD, MINT, GOLD, CREAM, MUTED, WHITE, DISPLAY, COND, BODY,
  Fonts, Backdrop, Phone, Footage, Lens, Rise, Tag, Ghost, Chip, Wipe, Flash, Callout, Frame, Sfx,
  PHONE_ASPECT, alpha, clamp, hold, inExpo, inOut, lerpPose, mix, outExpo, pop, ramp,
  type Pose, type Shot,
} from "./lib";
import { SHOTS } from "./shots";

// ---------------------------------------------------------------------------
// timeline (seconds). The music is 120 BPM: a bar is 2 s, a phrase 8 s; the
// track's drop sits at T.reveal and its final hit at T.end.
// ---------------------------------------------------------------------------
const MUSIC_AT = 12.03; // where the trailer starts inside the track
const T = { reveal: 4, wild: 12, storm: 20, ember: 24, warden: 28, thumbs: 32, desk: 36, desk2: 40, montage: 44, end: 52 };
export const TOTAL = 56.6;

const BUILD = "Beta 0.41";
const PHONE_NOTE = "Phone interface · rendered by the desktop build";
const DESK_NOTE = "Desktop build";

// One accent per hero class.
const PINK = "#FF4D8D";
const VOLT = "#3FD8FF";
const EMBER = "#FF7A2F";
const LEAF = "#8BE04E";
const DAWN = "#FFD76A";
const FROST = "#9AD8FF";

const CLASSES = ["Warrior", "Mage", "Ranger", "Cleric", "Warden", "Dawnweaver", "Wildspark", "Cinderforge", "Edgeweaver",
  "Stormfist", "Veilstalker", "Emberveil", "Orbitwright", "Riftshot", "Chainkeeper", "Frostguard", "Adventurer"];

// A point of the phone screen (0..1) on the canvas, for an unrotated pose.
const onPhone = (pose: Pose, u: number, v: number) => ({
  x: pose.x + (u - 0.5) * pose.w, y: pose.y + (v - 0.5) * (pose.w / PHONE_ASPECT),
});
const float = (t: number, pose: Pose, amount = 1): Pose => ({
  ...pose, y: pose.y + Math.sin(t * 1.3) * 5 * amount, ry: (pose.ry ?? 0) + Math.sin(t * 0.9) * 1.1 * amount,
  rx: (pose.rx ?? 0) + Math.cos(t * 0.7) * 0.6 * amount,
});

// ---------------------------------------------------------------------------
// 0. cold open
// ---------------------------------------------------------------------------
const Intro: React.FC = () => {
  const t = useT();
  const bars = ramp(t, 3.5, 3.98, inExpo);
  const line = ramp(t, 0.1, 1.2);
  return (
    <AbsoluteFill>
      <Backdrop accent={EMERALD} />
      <Ghost text="OMOBA" y={590} size={420} opacity={0.07} speed={60} />
      <div style={{ position: "absolute", left: 0, right: 0, top: 300, display: "flex", flexDirection: "column", alignItems: "center", gap: 26 }}>
        <Tag text={`Open-source MOBA · ${BUILD}`} at={0.15} out={3.3} accent={GOLD} size={32} />
        <Rise text="A WHOLE MOBA" at={0.45} out={3.3} size={138} />
        <Rise text="IN YOUR POCKET" at={1.45} out={3.36} size={138} color={MINT} />
      </div>
      <div style={{ position: "absolute", left: W / 2 - 420 * line, top: 760, width: 840 * line, height: 3, background: `linear-gradient(90deg, transparent, ${GOLD}, transparent)`, opacity: 1 - ramp(t, 3.2, 3.5) }} />
      {/* the frame closes on the riser */}
      <div style={{ position: "absolute", left: 0, right: 0, top: 0, height: bars * H * 0.5, background: INK }} />
      <div style={{ position: "absolute", left: 0, right: 0, bottom: 0, height: bars * H * 0.5, background: INK }} />
    </AbsoluteFill>
  );
};

// ---------------------------------------------------------------------------
// 1. reveal: the phone arrives with the hero roster
// ---------------------------------------------------------------------------
const Ticker: React.FC<{ at: number; out: number }> = ({ at, out }) => {
  const t = useT();
  const row = 54;
  const pos = interpolate(t, [at + 0.3, out - 0.3], [0, CLASSES.length - 1], clamp);
  const o = hold(t, at, out, 0.3);
  return (
    <div style={{ position: "absolute", right: 112, top: 96, width: 460, height: row * 3, overflow: "hidden", opacity: o,
      maskImage: "linear-gradient(180deg, transparent, black 30%, black 70%, transparent)", WebkitMaskImage: "linear-gradient(180deg, transparent, black 30%, black 70%, transparent)" }}>
      <div style={{ transform: `translateY(${row - pos * row}px)` }}>
        {CLASSES.map((name, i) => {
          const near = 1 - Math.min(1, Math.abs(i - pos));
          return (
            <div key={name} style={{ height: row, display: "flex", justifyContent: "flex-end", alignItems: "center", gap: 18 }}>
              <span style={{ fontFamily: COND, fontWeight: 600, fontSize: 26, letterSpacing: "0.2em", color: alpha(GOLD, 0.5 + near * 0.5) }}>{String(i + 1).padStart(2, "0")}</span>
              <span style={{ fontFamily: DISPLAY, fontWeight: 600, fontSize: mix(near, 28, 36), color: alpha(CREAM, 0.35 + near * 0.65), textTransform: "uppercase" }}>{name}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
};

const Reveal: React.FC = () => {
  const t = useT();
  const far: Pose = { x: 960, y: 1560, w: 1120, rx: 42, ry: -26, rz: -7 };
  const hero: Pose = { x: 960, y: 668, w: 1240, rx: 9, ry: -11, rz: -1.5 };
  const close: Pose = { x: 960, y: 652, w: 1440, rx: 2, ry: 0, rz: 0 };
  const push = ramp(t, 4.9, 5.9, inOut);
  const pose = float(t, lerpPose(push, lerpPose(pop(t, 0.02, 15, 120), far, hero), close), 1 - push * 0.6);
  return (
    <AbsoluteFill>
      <Backdrop shot={SHOTS.roster} accent={EMERALD} dim={0.38} />
      <Ghost text="OMOBA" y={300} size={400} opacity={0.08} />
      <div style={{ position: "absolute", left: 112, top: 84 }}>
        <Tag text="Choose your hero" at={0.3} out={4.7} accent={GOLD} />
        <div style={{ display: "flex", gap: 30, marginTop: 22 }}>
          <Rise text="17" at={0.45} out={4.7} size={124} color={MINT} />
          <Rise text="CLASSES" at={0.55} out={4.75} size={124} />
        </div>
      </div>
      <Ticker at={0.6} out={4.9} />
      <Phone pose={pose} accent={EMERALD} sheen={ramp(t, 0.7, 1.9, inOut)}>
        <Footage shot={SHOTS.roster} />
      </Phone>
      {/* second half: what is inside */}
      <div style={{ position: "absolute", left: 0, right: 0, top: 92, display: "flex", justifyContent: "center", gap: 22 }}>
        {["5v5 matches", "3 lanes", "Jungle camps", "Towers + Nexus"].map((text, i) => (
          <Chip key={text} text={text} at={5.5 + i * 0.16} color={i === 0 ? MINT : alpha(CREAM, 0.12)} ink={i === 0 ? INK : CREAM} size={34}
            style={{ border: i === 0 ? "none" : `2px solid ${alpha(CREAM, 0.35)}` }} />
        ))}
      </div>
      <Frame accent={GOLD} label={`OMOBA // ${BUILD}`} right={PHONE_NOTE} />
    </AbsoluteFill>
  );
};

// ---------------------------------------------------------------------------
// 2. a hero class on the phone: name, role, the four abilities
// ---------------------------------------------------------------------------
type ClassCard = { name: string; role: string; index: number; skills: string[]; accent: string; shot: Shot; flip?: boolean };

/** One shot, or a cut to a second one at `cutAt` seconds. */
const Cut: React.FC<{ shot: Shot; next?: Shot; cutAt?: number }> = ({ shot, next, cutAt = 0 }) => (
  next ? (
    <>
      <Sequence durationInFrames={F(cutAt)} layout="none"><Footage shot={shot} /></Sequence>
      <Sequence from={F(cutAt)} layout="none"><Footage shot={next} /></Sequence>
    </>
  ) : <Footage shot={shot} />
);

const SkillList: React.FC<{ card: ClassCard; x: number; y: number; at: number; out: number; right?: boolean }> = ({ card, x, y, at, out, right }) => {
  const t = useT();
  return (
    <div style={{ position: "absolute", top: y, ...(right ? { right: W - x } : { left: x }), width: 520 }}>
      {card.skills.map((skill, i) => {
        const a = at + i * 0.11;
        const p = ramp(t, a, a + 0.5);
        const o = 1 - ramp(t, out + i * 0.04, out + 0.25 + i * 0.04, inExpo);
        return (
          <div key={skill} style={{ height: 62, opacity: Math.min(1, p * 1.5) * o, transform: `translateX(${(1 - p) * (right ? 70 : -70)}px)`, textAlign: right ? "right" : "left" }}>
            <div style={{ display: "flex", alignItems: "baseline", gap: 16, flexDirection: right ? "row-reverse" : "row" }}>
              <span style={{ fontFamily: COND, fontWeight: 700, fontSize: 24, letterSpacing: "0.18em", color: card.accent }}>{String(i + 1).padStart(2, "0")}</span>
              <span style={{ fontFamily: DISPLAY, fontWeight: 600, fontSize: 32, color: CREAM, whiteSpace: "nowrap" }}>{skill}</span>
            </div>
            <div style={{ height: 2, marginTop: 9, background: alpha(CREAM, 0.22), transformOrigin: right ? "100% 50%" : "0% 50%", transform: `scaleX(${p})` }} />
          </div>
        );
      })}
    </div>
  );
};

const ClassScene: React.FC<{ card: ClassCard; dur: number; deep?: React.ReactNode; deepAt?: number; deepShot?: Shot }> = ({ card, dur, deep, deepAt, deepShot }) => {
  const t = useT();
  const flip = !!card.flip;
  const side = flip ? -1 : 1; // +1: phone on the right, text on the left
  const titleOut = deepAt ?? dur - 0.35;
  const rest: Pose = { x: 960 + side * 300, y: 690, w: 1120, rx: 6, ry: -9 * side, rz: -1.2 * side };
  const from: Pose = { ...rest, x: rest.x + side * 900, ry: -34 * side, rz: -6 * side };
  const wide: Pose = { x: 960, y: 600, w: 1500, rx: 0, ry: 0, rz: 0 };
  const push = deepAt === undefined ? 0 : ramp(t, deepAt, deepAt + 0.9, inOut);
  const pose = float(t, lerpPose(push, lerpPose(pop(t, 0.08, 16, 130), from, rest), wide), 1 - push);
  return (
    <AbsoluteFill>
      {deepShot && deepAt !== undefined ? (
        <>
          <Sequence durationInFrames={F(deepAt + 0.3)}><Backdrop shot={card.shot} accent={card.accent} /></Sequence>
          <Sequence from={F(deepAt + 0.3)}><Backdrop shot={deepShot} accent={card.accent} /></Sequence>
        </>
      ) : <Backdrop shot={card.shot} accent={card.accent} />}
      <Ghost text={card.name} y={flip ? 640 : 250} size={340} color={card.accent} opacity={0.13} speed={flip ? 120 : 80} />
      <Phone pose={pose} accent={card.accent} sheen={ramp(t, 0.5, 1.5, inOut)}>
        <Cut shot={card.shot} next={deepShot} cutAt={(deepAt ?? 0) + 0.3} />
        {deepShot && deepAt !== undefined && <Flash at={deepAt + 0.3} strength={0.7} len={0.3} />}
      </Phone>
      {/* index, name, role */}
      <div style={{ position: "absolute", top: 104, ...(flip ? { right: 112, textAlign: "right" } : { left: 112 }), display: "flex", flexDirection: "column", alignItems: flip ? "flex-end" : "flex-start" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 22, opacity: Math.min(ramp(t, 0.2, 0.7), 1 - ramp(t, titleOut, titleOut + 0.25)), flexDirection: flip ? "row-reverse" : "row" }}>
          <span style={{ fontFamily: DISPLAY, fontWeight: 600, fontSize: 34, color: card.accent }}>{String(card.index).padStart(2, "0")}<span style={{ color: alpha(CREAM, 0.45) }}> / 17</span></span>
          <span style={{ width: 90 * ramp(t, 0.3, 1.0), height: 3, background: card.accent }} />
          <span style={{ fontFamily: COND, fontWeight: 700, fontSize: 30, letterSpacing: "0.2em", color: CREAM, textTransform: "uppercase" }}>{card.role}</span>
        </div>
        <Rise text={card.name.toUpperCase()} at={0.22} out={titleOut} size={card.name.length > 9 ? 124 : 138} style={{ marginTop: 20 }} />
      </div>
      <SkillList card={card} x={flip ? W - 112 : 112} y={336} at={0.7} out={titleOut - 0.1} right={flip} />
      {/* the same footage, close on the hero */}
      <div style={{ position: "absolute", inset: 0, opacity: 1 - ramp(t, titleOut - 0.1, titleOut + 0.2, inExpo) }}>
        <Lens shot={card.shot} x={flip ? W - 330 : 330} y={798} d={372} accent={card.accent} at={1.0} u={0.5} v={0.555} span={0.17} />
      </div>
      {deep}
      <Frame accent={card.accent} label={`OMOBA // ${BUILD}`} right={PHONE_NOTE} />
    </AbsoluteFill>
  );
};

const WILDSPARK: ClassCard = { name: "Wildspark", role: "Explosive marksman", index: 7, accent: PINK, shot: SHOTS.wildspark,
  skills: ["Switchfire", "Shockline", "Snapline", "Last Spark"] };
const STORMFIST: ClassCard = { name: "Stormfist", role: "Energy fighter", index: 10, accent: VOLT, shot: SHOTS.stormfist, flip: true,
  skills: ["Echo Strike", "Anchor Step", "Thunder Pulse", "Thunder Kick"] };
const EMBERVEIL: ClassCard = { name: "Emberveil", role: "Orb, charm, fiery dashes", index: 12, accent: EMBER, shot: SHOTS.emberveil,
  skills: ["Wandering Ember", "Kindled Wisps", "Heart Tether", "Flame Dance"] };
const WARDEN: ClassCard = { name: "Warden", role: "Jungler", index: 5, accent: LEAF, shot: SHOTS.warden, flip: true,
  skills: ["Feral Swipe", "Barkskin", "Hunter's Mark", "Primal Maul"] };

const WildsparkDeep: React.FC = () => {
  const wide: Pose = { x: 960, y: 600, w: 1500 };
  const rocket = onPhone(wide, 0.853, 0.496), traps = onPhone(wide, 0.765, 0.508), swap = onPhone(wide, 0.697, 0.838);
  return (
    <>
      <Callout at={5.0} out={7.5} x={rocket.x} y={rocket.y} lx={rocket.x + 40} ly={160} title="Last Spark" sub="Long-range finisher" accent={PINK} align="right" />
      <Callout at={5.35} out={7.5} x={traps.x} y={traps.y} lx={traps.x - 330} ly={160} title="Snapline" sub="Armed traps" accent={GOLD} align="right" />
      <Callout at={5.7} out={7.5} x={swap.x} y={swap.y} lx={swap.x - 120} ly={1010} title="Switchfire" sub="Repeater ⇄ rockets" accent={MINT} align="right" />
    </>
  );
};

// ---------------------------------------------------------------------------
// 3. thumbs: what each hand does
// ---------------------------------------------------------------------------
const Thumbs: React.FC = () => {
  const t = useT();
  const pose: Pose = { x: 960, y: 610, w: 1180, rx: 0, ry: 0, rz: 0 };
  const inP = lerpPose(pop(t, 0.05, 16, 130), { ...pose, y: 1500, rx: 30 }, pose);
  const stick = onPhone(pose, 0.118, 0.781), attack = onPhone(pose, 0.814, 0.752);
  return (
    <AbsoluteFill>
      <Backdrop shot={SHOTS.thumbs} accent={MINT} />
      <div style={{ position: "absolute", left: 0, right: 0, top: 78, display: "flex", flexDirection: "column", alignItems: "center", gap: 18 }}>
        <Tag text="Phone controls" at={0.2} accent={GOLD} />
        <Rise text="BUILT FOR THUMBS" at={0.3} size={112} />
      </div>
      <Phone pose={inP} accent={MINT} sheen={ramp(t, 0.5, 1.4, inOut)}>
        <Footage shot={SHOTS.thumbs} />
      </Phone>
      <Callout at={1.0} out={3.7} x={stick.x} y={stick.y} lx={stick.x} ly={985} title="Left thumb moves" sub="Analog stick" accent={MINT} />
      <Callout at={1.35} out={3.7} x={attack.x} y={attack.y} lx={attack.x} ly={985} title="Right thumb fights" sub="Attack + four abilities" accent={GOLD} />
      <Frame accent={MINT} label={`OMOBA // ${BUILD}`} right={PHONE_NOTE} />
    </AbsoluteFill>
  );
};

// ---------------------------------------------------------------------------
// 4. the same game in a desktop window
// ---------------------------------------------------------------------------
/** The capture in a desktop window; `x`/`y` is the centre of the window. */
const DeskWindow: React.FC<{ shot: Shot; x: number; y: number; w: number; ry?: number; rx?: number; accent: string }> = ({ shot, x, y, w, ry = 0, rx = 3, accent }) => {
  const h = (w * 9) / 16, bar = Math.round(w * 0.033);
  return (
    <div style={{ position: "absolute", inset: 0, perspective: 2600 }}>
      <div style={{ position: "absolute", left: x - w / 2, top: y - (h + bar) / 2, width: w, height: h + bar, transform: `rotateY(${ry}deg) rotateX(${rx}deg)`, transformStyle: "preserve-3d" }}>
        <div style={{ position: "absolute", inset: -70, borderRadius: 60, background: alpha(accent, 0.3), filter: "blur(90px)" }} />
        <div style={{ position: "absolute", inset: 0, borderRadius: 20, overflow: "hidden", background: "#0b1110", boxShadow: `0 0 0 2px ${alpha(WHITE, 0.16)}, 0 50px 140px rgba(0,0,0,0.7)` }}>
          <div style={{ height: bar, display: "flex", alignItems: "center", gap: 9, padding: "0 18px", background: "linear-gradient(180deg, #1a2422, #101816)" }}>
            {["#FF5F57", "#FEBC2E", "#28C840"].map((c) => <div key={c} style={{ width: 13, height: 13, borderRadius: 7, background: c }} />)}
            <div style={{ flex: 1, textAlign: "center", fontFamily: COND, fontWeight: 600, fontSize: 19, letterSpacing: "0.2em", color: alpha(CREAM, 0.6), textTransform: "uppercase" }}>Omoba · desktop</div>
            <div style={{ width: 57 }} />
          </div>
          <div style={{ width: w, height: h }}><Footage shot={shot} /></div>
        </div>
      </div>
    </div>
  );
};

const Desk: React.FC<{ shot: Shot; name: string; role: string; accent: string; headline: string; flip?: boolean; first?: boolean }> = ({ shot, name, role, accent, headline, flip, first }) => {
  const t = useT();
  const side = flip ? -1 : 1;
  const p = pop(t, 0.05, 16, 130);
  const ry = mix(p, -30 * side, -5 * side) + Math.sin(t * 0.9) * 0.8;
  return (
    <AbsoluteFill>
      <Backdrop shot={shot} accent={accent} />
      <Ghost text="DESKTOP" y={640} size={330} color={accent} opacity={0.1} />
      <div style={{ position: "absolute", left: 0, right: 0, top: 70, display: "flex", flexDirection: "column", alignItems: "center", gap: 16 }}>
        <Tag text={first ? "Not only phones" : "Mouse + QWER"} at={0.2} accent={GOLD} />
        <Rise text={headline} at={0.3} size={96} />
      </div>
      <DeskWindow shot={shot} x={W / 2 + (1 - p) * side * 700} y={602} w={1160} ry={ry} accent={accent} />
      <div style={{ position: "absolute", left: 0, right: 0, top: 972, display: "flex", justifyContent: "center", alignItems: "center", gap: 20 }}>
        <Chip text={name} at={0.7} color={accent} size={30} />
        <Tag text={role} at={0.85} accent={accent} size={26} />
      </div>
      <Frame accent={accent} label={`OMOBA // ${BUILD}`} right={DESK_NOTE} />
    </AbsoluteFill>
  );
};

// ---------------------------------------------------------------------------
// 5. montage: one cut a second, one word a cut
// ---------------------------------------------------------------------------
const WORDS = ["PUSH", "LANES", "CLEAR", "CAMPS", "LAND", "COMBOS", "TAKE", "TOWERS"];
const HUES = [PINK, VOLT, LEAF, LEAF, DAWN, VOLT, DAWN, PINK];
const Montage: React.FC = () => {
  const t = useT();
  const i = Math.min(WORDS.length - 1, Math.floor(t));
  const lt = t - i;
  const shot = SHOTS.montage[i];
  const phone = shot.clip.startsWith("phone");
  const accent = HUES[i];
  const word = WORDS[i];
  const outline = i % 2 === 1;
  const size = Math.min(300, 1560 / (word.length * 0.84));
  const slam = ramp(lt, 0, 0.42);
  const side = i % 2 ? 1 : -1;
  const kick = 1 + 0.07 * (1 - ramp(lt, 0, 0.5));
  const pose: Pose = { x: 960, y: 706, w: 1240 * kick, rx: 7, ry: side * mix(ramp(lt, 0, 1, outExpo), 13, 6), rz: side * -1 };
  return (
    <AbsoluteFill style={{ background: INK, overflow: "hidden" }}>
      <Sequence from={F(i)} durationInFrames={F(1) + 1}>
        <Backdrop shot={shot} accent={accent} dim={0.34} />
      </Sequence>
      <div style={{ position: "absolute", left: 0, right: 0, top: 58, display: "flex", justifyContent: "center" }}>
        <div style={{ fontFamily: DISPLAY, fontWeight: 900, fontSize: size, letterSpacing: "-0.03em", lineHeight: 1, whiteSpace: "nowrap", transformOrigin: "50% 60%",
          transform: `scale(${mix(slam, 1.5, 1)}) rotate(${side * mix(slam, 5, 1.2)}deg)`, opacity: Math.min(1, lt * 14),
          color: outline ? "transparent" : CREAM, WebkitTextStroke: outline ? `4px ${accent}` : undefined, textShadow: outline ? "none" : `0 12px 50px ${alpha(INK, 0.7)}` }}>{word}</div>
      </div>
      <Sequence from={F(i)} durationInFrames={F(1) + 1} layout="none">
        {phone
          ? <Phone pose={pose} accent={accent}><Footage shot={shot} /></Phone>
          : <DeskWindow shot={shot} x={960} y={690} w={1000 * kick} ry={pose.ry} rx={5} accent={accent} />}
      </Sequence>
      <AbsoluteFill style={{ background: WHITE, opacity: 0.5 * (1 - ramp(lt, 0, 0.2)) }} />
      <Frame accent={accent} label={`OMOBA // ${BUILD}`} right={phone ? PHONE_NOTE : DESK_NOTE} />
    </AbsoluteFill>
  );
};

// ---------------------------------------------------------------------------
// 6. end card
// ---------------------------------------------------------------------------
const End: React.FC = () => {
  const t = useT();
  const bar = ramp(t, 0.5, 1.3);
  const link = (text: string, at: number, main = false) => {
    const s = pop(t, at);
    return (
      <div key={text} style={{ padding: "18px 40px", borderRadius: 60, background: main ? MINT : alpha(CREAM, 0.08), border: main ? "none" : `2px solid ${alpha(CREAM, 0.3)}`,
        color: main ? INK : CREAM, fontFamily: BODY, fontWeight: 800, fontSize: main ? 46 : 36, transform: `scale(${s})`, opacity: Math.min(1, s * 2), whiteSpace: "nowrap" }}>{text}</div>
    );
  };
  return (
    <AbsoluteFill>
      <Backdrop shot={SHOTS.end} accent={EMERALD} dim={0.26} />
      <Ghost text="PLAY · CREATE · CONTRIBUTE" y={760} size={230} opacity={0.07} speed={110} />
      <div style={{ position: "absolute", left: 0, right: 0, top: 200, display: "flex", flexDirection: "column", alignItems: "center" }}>
        <Tag text="Open-source MOBA · Rust + Bevy" at={0.35} accent={GOLD} size={32} />
        <Rise text="OMOBA" at={0.05} size={290} stagger={0.05} style={{ marginTop: 26 }} />
        <div style={{ width: 760 * bar, height: 6, marginTop: 24, background: `linear-gradient(90deg, ${MINT}, ${GOLD})` }} />
        <div style={{ display: "flex", gap: 26, marginTop: 56, alignItems: "center" }}>
          {link("omoba.io", 0.9, true)}
          {link("discord.gg/DMhvaVpj7Q", 1.05)}
          {link("github.com/o-moba", 1.2)}
        </div>
        <div style={{ marginTop: 44, fontFamily: COND, fontWeight: 600, fontSize: 26, letterSpacing: "0.2em", color: alpha(CREAM, 0.6), textTransform: "uppercase", opacity: ramp(t, 1.5, 2.1) }}>
          {BUILD} · gameplay captured in offline practice with bots
        </div>
      </div>
      <Frame accent={GOLD} label={`OMOBA // ${BUILD}`} />
    </AbsoluteFill>
  );
};

// ---------------------------------------------------------------------------
// the cut
// ---------------------------------------------------------------------------
const Scene: React.FC<{ at: number; to: number; children: React.ReactNode }> = ({ at, to, children }) => (
  <Sequence from={F(at)} durationInFrames={F(to - at)}>{children}</Sequence>
);

export const Trailer: React.FC = () => {
  const t = useT();
  const musicVol = (f: number) => {
    return 0.8 * interpolate(f / FPS, [0, 0.25, TOTAL - 1.4, TOTAL - 0.05], [0, 1, 1, 0], clamp);
  };
  const wipes: [number, [string, string, string], 1 | -1][] = [
    [T.wild, [INK, GOLD, PINK], 1], [T.storm, [INK, CREAM, VOLT], -1], [T.ember, [INK, GOLD, EMBER], 1],
    [T.warden, [INK, CREAM, LEAF], -1], [T.thumbs, [INK, GOLD, MINT], 1], [T.desk, [INK, CREAM, DAWN], -1],
    [T.desk2, [INK, CREAM, FROST], 1], [T.montage, [INK, MINT, GOLD], -1],
  ];
  return (
    <AbsoluteFill style={{ background: INK }}>
      <Fonts />
      <Audio src={staticFile("music/v2.mp3")} startFrom={F(MUSIC_AT)} volume={musicVol} />
      <Scene at={0} to={T.reveal}><Intro /></Scene>
      <Scene at={T.reveal} to={T.wild}><Reveal /></Scene>
      <Scene at={T.wild} to={T.storm}><ClassScene card={WILDSPARK} dur={8} deepAt={4.3} deep={<WildsparkDeep />} deepShot={SHOTS.wildsparkPush} /></Scene>
      <Scene at={T.storm} to={T.ember}><ClassScene card={STORMFIST} dur={4} /></Scene>
      <Scene at={T.ember} to={T.warden}><ClassScene card={EMBERVEIL} dur={4} /></Scene>
      <Scene at={T.warden} to={T.thumbs}><ClassScene card={WARDEN} dur={4} /></Scene>
      <Scene at={T.thumbs} to={T.desk}><Thumbs /></Scene>
      <Scene at={T.desk} to={T.desk2}><Desk shot={SHOTS.dawnweaver} name="Dawnweaver" role="Light mage" accent={DAWN} headline="SAME GAME ON DESKTOP" first /></Scene>
      <Scene at={T.desk2} to={T.montage}><Desk shot={SHOTS.frostguard} name="Frostguard" role="Ally protector" accent={FROST} headline="ALL 17 CLASSES HERE TOO" flip /></Scene>
      <Scene at={T.montage} to={T.end}><Montage /></Scene>
      <Scene at={T.end} to={TOTAL}><End /></Scene>
      {wipes.map(([at, colors, dir]) => <Wipe key={at} at={at} colors={colors} dir={dir} />)}
      <Flash at={T.reveal} strength={0.9} len={0.45} />
      <Flash at={T.end} strength={0.9} len={0.5} />
      {/* sound */}
      <Sfx at={2.3} name="riser" vol={0.22} />
      <Sfx at={T.reveal} name="boom" vol={0.4} />
      {wipes.map(([at]) => <Sfx key={at} at={at - 0.3} name="whoosh" vol={0.22} />)}
      {[0, 1, 2, 3].map((i) => <Sfx key={i} at={T.reveal + 5.5 + i * 0.16} name="pop" vol={0.14} />)}
      <Sfx at={T.end} name="boom" vol={0.45} />
      {/* fade to black on the tail */}
      <AbsoluteFill style={{ background: "#000", opacity: ramp(t, TOTAL - 0.7, TOTAL - 0.05, inOut) }} />
    </AbsoluteFill>
  );
};
