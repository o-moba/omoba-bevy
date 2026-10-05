import React from "react";
import {
  AbsoluteFill, Audio, Easing, Img, Sequence, interpolate, spring, staticFile, useCurrentFrame,
} from "remotion";

// ---------------------------------------------------------------------------
// canvas, clock
// ---------------------------------------------------------------------------
export const FPS = 60;
export const W = 1920;
export const H = 1080;
export const F = (s: number) => Math.round(s * FPS);
export const useT = () => useCurrentFrame() / FPS;

// ---------------------------------------------------------------------------
// look: the game's own palette (deep forest glass, mint, gold) plus one accent per hero class
// ---------------------------------------------------------------------------
export const INK = "#040A09";
export const DEEP = "#081A17";
export const TEAL = "#0F3A33";
export const EMERALD = "#19B892";
export const MINT = "#5BF0C0";
export const GOLD = "#E9C46A";
export const CREAM = "#F6EFD9";
export const MUTED = "#8FA8A0";
export const WHITE = "#FFFFFF";

export const DISPLAY = "Unbounded";
export const COND = "BarlowCondensed";
export const BODY = "Manrope";

export const Fonts: React.FC = () => (
  <style>{`
    @font-face { font-family: ${DISPLAY}; src: url(${staticFile("fonts/VPUnboundedBlack.ttf")}); font-weight: 900; }
    @font-face { font-family: ${DISPLAY}; src: url(${staticFile("fonts/VPUnboundedSemiBold.ttf")}); font-weight: 600; }
    @font-face { font-family: ${COND}; src: url(${staticFile("fonts/BarlowCondensed-Bold.ttf")}); font-weight: 700; }
    @font-face { font-family: ${COND}; src: url(${staticFile("fonts/BarlowCondensed-SemiBold.ttf")}); font-weight: 600; }
    @font-face { font-family: ${BODY}; src: url(${staticFile("fonts/VPManropeExtraBold.ttf")}); font-weight: 800; }
  `}</style>
);

// ---------------------------------------------------------------------------
// motion helpers
// ---------------------------------------------------------------------------
export const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;
export const outExpo = Easing.bezier(0.16, 1, 0.3, 1);
export const inOut = Easing.bezier(0.65, 0, 0.35, 1);
export const inExpo = Easing.bezier(0.7, 0, 0.84, 0);
/** eased 0..1 between a and b seconds */
export const ramp = (t: number, a: number, b: number, easing = outExpo) =>
  interpolate(t, [a, b], [0, 1], { ...clamp, easing });
export const mix = (p: number, v0: number, v1: number) => v0 + (v1 - v0) * p;
/** snappy overshooting 0..1 that starts at `at` seconds */
export const pop = (t: number, at: number, damping = 13, stiffness = 170) =>
  t < at ? 0 : spring({ frame: (t - at) * FPS, fps: FPS, config: { damping, stiffness, mass: 0.7 } });
/** 1 inside [a, b], with eased edges of `edge` seconds */
export const hold = (t: number, a: number, b: number, edge = 0.25) =>
  Math.min(ramp(t, a, a + edge), 1 - ramp(t, b - edge, b, inExpo));
export const alpha = (hex: string, a: number) => {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${a})`;
};

// ---------------------------------------------------------------------------
// gameplay sources: still frames public/gp/<clip>/<frame>.jpg extracted by
// prepare.py (frame n of a 60 fps clip). A shot shows `from` at the first frame
// of its sequence and advances `rate` captured frames per frame, so every
// frame of the video is exactly one captured frame.
// ---------------------------------------------------------------------------
export type Shot = { clip: string; from: number; len: number; rate?: number };
export const Footage: React.FC<{ shot: Shot; style?: React.CSSProperties }> = ({ shot, style }) => {
  const frame = useCurrentFrame();
  const first = Math.round(shot.from * FPS);
  const last = first + Math.round(shot.len * FPS);
  const index = Math.min(last, Math.max(first, first + Math.round(frame * (shot.rate ?? 1))));
  return (
    <Img src={staticFile(`gp/${shot.clip}/${String(index).padStart(6, "0")}.jpg`)}
      style={{ width: "100%", height: "100%", objectFit: "cover", display: "block", ...style }} />
  );
};

// ---------------------------------------------------------------------------
// backdrop: blurred footage, colour wash, drifting grid, light streaks, vignette
// ---------------------------------------------------------------------------
export const Backdrop: React.FC<{ shot?: Shot; accent: string; dim?: number }> = ({ shot, accent, dim = 0.3 }) => {
  const t = useT();
  return (
    <AbsoluteFill style={{ background: INK, overflow: "hidden" }}>
      {shot && (
        <AbsoluteFill style={{ transform: `scale(${1.25 + t * 0.006})`, filter: `blur(46px) saturate(1.5) brightness(${dim})` }}>
          <Footage shot={shot} />
        </AbsoluteFill>
      )}
      <AbsoluteFill style={{ background: `radial-gradient(120% 90% at 50% 45%, ${alpha(DEEP, 0)} 0%, ${alpha(INK, 0.55)} 70%, ${INK} 100%)` }} />
      <AbsoluteFill style={{ background: `radial-gradient(60% 55% at ${50 + Math.sin(t * 0.5) * 6}% 58%, ${alpha(accent, 0.2)} 0%, ${alpha(accent, 0)} 70%)` }} />
      {/* drifting dot grid */}
      <AbsoluteFill style={{
        backgroundImage: `radial-gradient(${alpha(CREAM, 0.13)} 1.6px, transparent 1.7px)`, backgroundSize: "48px 48px",
        backgroundPosition: `${-t * 14}px ${t * 9}px`,
        maskImage: "radial-gradient(90% 80% at 50% 50%, black 30%, transparent 85%)",
        WebkitMaskImage: "radial-gradient(90% 80% at 50% 50%, black 30%, transparent 85%)",
      }} />
      {/* light streaks */}
      {[0, 1, 2].map((i) => {
        const x = ((t * (70 + i * 26) + i * 760) % (W + 900)) - 450;
        return <div key={i} style={{
          position: "absolute", left: x, top: -200, width: 2 + i, height: H + 400, transform: "rotate(24deg)",
          background: `linear-gradient(180deg, transparent, ${alpha(i === 1 ? accent : MINT, 0.35)}, transparent)`,
        }} />;
      })}
      <AbsoluteFill style={{ boxShadow: "inset 0 0 260px rgba(0,0,0,0.75)" }} />
    </AbsoluteFill>
  );
};

// ---------------------------------------------------------------------------
// phone: a landscape device around the 844x390 phone interface capture
// ---------------------------------------------------------------------------
export const PHONE_ASPECT = 2532 / 1170;
export type Pose = { x: number; y: number; w: number; rx?: number; ry?: number; rz?: number };
export const lerpPose = (p: number, a: Pose, b: Pose): Pose => ({
  x: mix(p, a.x, b.x), y: mix(p, a.y, b.y), w: mix(p, a.w, b.w),
  rx: mix(p, a.rx ?? 0, b.rx ?? 0), ry: mix(p, a.ry ?? 0, b.ry ?? 0), rz: mix(p, a.rz ?? 0, b.rz ?? 0),
});
/** `pose.x/y` is the centre of the screen on the canvas, `pose.w` the screen width. */
export const Phone: React.FC<{ pose: Pose; accent: string; children: React.ReactNode; sheen?: number; overlay?: React.ReactNode }> = ({ pose, accent, children, sheen = -1, overlay }) => {
  const sw = pose.w, sh = sw / PHONE_ASPECT;
  const bezel = sw * 0.016, radius = sw * 0.056;
  const ow = sw + bezel * 2, oh = sh + bezel * 2;
  return (
    <div style={{ position: "absolute", left: 0, top: 0, width: W, height: H, perspective: 2600, perspectiveOrigin: "50% 50%", pointerEvents: "none" }}>
      <div style={{
        position: "absolute", left: pose.x - ow / 2, top: pose.y - oh / 2, width: ow, height: oh, transformStyle: "preserve-3d",
        transform: `rotateX(${pose.rx ?? 0}deg) rotateY(${pose.ry ?? 0}deg) rotateZ(${pose.rz ?? 0}deg)`,
      }}>
        {/* glow and contact shadow */}
        <div style={{ position: "absolute", inset: -sw * 0.05, borderRadius: radius * 2, background: alpha(accent, 0.34), filter: `blur(${sw * 0.07}px)` }} />
        <div style={{ position: "absolute", left: sw * 0.04, right: sw * 0.04, top: oh * 0.72, height: oh * 0.5, borderRadius: "50%", background: "rgba(0,0,0,0.6)", filter: `blur(${sw * 0.05}px)` }} />
        {/* body */}
        <div style={{
          position: "absolute", inset: 0, borderRadius: radius + bezel,
          background: "linear-gradient(145deg, #2c3432 0%, #0b0e0e 28%, #050606 60%, #1c2221 100%)",
          boxShadow: `0 0 0 ${Math.max(1.5, sw * 0.0016)}px ${alpha(WHITE, 0.2)}, inset 0 0 ${sw * 0.01}px rgba(255,255,255,0.22), 0 ${sw * 0.03}px ${sw * 0.09}px rgba(0,0,0,0.65)`,
        }} />
        {/* side buttons */}
        <div style={{ position: "absolute", left: ow * 0.2, top: -sw * 0.0035, width: ow * 0.075, height: sw * 0.005, borderRadius: 4, background: "#2c3432" }} />
        <div style={{ position: "absolute", left: ow * 0.3, top: -sw * 0.0035, width: ow * 0.05, height: sw * 0.005, borderRadius: 4, background: "#2c3432" }} />
        {/* screen */}
        <div style={{ position: "absolute", left: bezel, top: bezel, width: sw, height: sh, borderRadius: radius, overflow: "hidden", background: "#000" }}>
          {children}
          {overlay}
          {/* glass: static highlight and a moving sheen */}
          <div style={{ position: "absolute", inset: 0, background: "linear-gradient(118deg, rgba(255,255,255,0.10) 0%, rgba(255,255,255,0.02) 22%, transparent 40%)" }} />
          {sheen >= 0 && sheen <= 1 && (
            <div style={{ position: "absolute", top: -sh, left: mix(sheen, -sw * 0.5, sw * 1.2), width: sw * 0.22, height: sh * 3, transform: "rotate(20deg)", background: "linear-gradient(90deg, transparent, rgba(255,255,255,0.22), transparent)" }} />
          )}
          <div style={{ position: "absolute", inset: 0, borderRadius: radius, boxShadow: "inset 0 0 0 1.5px rgba(255,255,255,0.08)" }} />
        </div>
        {/* camera island on the short edge */}
        <div style={{ position: "absolute", left: bezel + sw * 0.008, top: bezel + sh * 0.5 - sh * 0.1, width: sw * 0.017, height: sh * 0.2, borderRadius: sw * 0.0085, background: "#000", boxShadow: "inset 0 0 3px rgba(255,255,255,0.12)" }} />
      </div>
    </div>
  );
};

/** A round close-up of the action: the capture cropped around (u, v) of the frame. */
export const Lens: React.FC<{ shot: Shot; x: number; y: number; d: number; accent: string; u?: number; v?: number; span?: number; aspect?: number; at?: number; label?: string }> = ({ shot, x, y, d, accent, u = 0.5, v = 0.47, span = 0.25, aspect = PHONE_ASPECT, at = 0, label }) => {
  const t = useT();
  const s = pop(t, at, 14, 150);
  const vw = d / span, vh = vw / aspect;
  const spin = t * 40;
  return (
    <div style={{ position: "absolute", left: x - d / 2, top: y - d / 2, width: d, height: d, transform: `scale(${s})`, opacity: Math.min(1, s * 1.6) }}>
      <div style={{ position: "absolute", inset: -26, borderRadius: "50%", background: alpha(accent, 0.3), filter: "blur(40px)" }} />
      <div style={{ position: "absolute", inset: 0, borderRadius: "50%", overflow: "hidden", background: "#000", boxShadow: `0 0 0 5px ${accent}, 0 30px 80px rgba(0,0,0,0.6)` }}>
        <div style={{ position: "absolute", left: d / 2 - u * vw, top: d / 2 - v * vh, width: vw, height: vh }}><Footage shot={shot} /></div>
        <div style={{ position: "absolute", inset: 0, borderRadius: "50%", boxShadow: "inset 0 0 60px rgba(0,0,0,0.45)" }} />
      </div>
      {/* a dashed ring turning around the lens */}
      <svg width={d + 44} height={d + 44} style={{ position: "absolute", left: -22, top: -22, transform: `rotate(${spin}deg)` }}>
        <circle cx={(d + 44) / 2} cy={(d + 44) / 2} r={d / 2 + 16} fill="none" stroke={alpha(CREAM, 0.55)} strokeWidth={2.5} strokeDasharray="4 18" />
      </svg>
      {label && (
        <div style={{ position: "absolute", left: 0, right: 0, bottom: -58, textAlign: "center", fontFamily: COND, fontWeight: 600, fontSize: 24, letterSpacing: "0.2em", color: alpha(CREAM, 0.8), textTransform: "uppercase", whiteSpace: "nowrap" }}>{label}</div>
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------
// typography
// ---------------------------------------------------------------------------
/** Letters rise out of a mask, one after another. */
export const Rise: React.FC<{ text: string; at: number; out?: number; size: number; color?: string; font?: string; weight?: number; tracking?: number; stagger?: number; style?: React.CSSProperties; stroke?: string }> = ({ text, at, out, size, color = CREAM, font = DISPLAY, weight = 900, tracking = -0.02, stagger = 0.028, style, stroke }) => {
  const t = useT();
  return (
    <div style={{ display: "flex", overflow: "hidden", padding: `${size * 0.08}px ${size * 0.04}px`, margin: `-${size * 0.08}px -${size * 0.04}px`, whiteSpace: "pre", ...style }}>
      {[...text].map((ch, i) => {
        const a = at + i * stagger;
        const p = ramp(t, a, a + 0.55);
        const q = out === undefined ? 0 : ramp(t, out + i * stagger * 0.5, out + i * stagger * 0.5 + 0.3, inExpo);
        return (
          <span key={i} style={{
            display: "inline-block", fontFamily: font, fontWeight: weight, fontSize: size, lineHeight: 1, letterSpacing: `${tracking}em`,
            color: stroke ? "transparent" : color, WebkitTextStroke: stroke ? `${Math.max(1.5, size * 0.012)}px ${stroke}` : undefined,
            transform: `translateY(${(1 - p) * 112 - q * 112}%) rotate(${(1 - p) * 7}deg)`, transformOrigin: "0% 100%",
            opacity: p <= 0 ? 0 : 1,
          }}>{ch}</span>
        );
      })}
    </div>
  );
};

/** Small condensed label with an accent tick; letters track in. */
export const Tag: React.FC<{ text: string; at: number; out?: number; color?: string; accent?: string; size?: number; style?: React.CSSProperties }> = ({ text, at, out, color = CREAM, accent = MINT, size = 30, style }) => {
  const t = useT();
  const p = ramp(t, at, at + 0.6);
  const o = out === undefined ? 1 : 1 - ramp(t, out, out + 0.25, inExpo);
  return (
    <div style={{ display: "flex", alignItems: "center", gap: size * 0.5, opacity: Math.min(p * 1.6, 1) * o, ...style }}>
      <div style={{ width: size * 0.42 * p, height: size * 0.42, background: accent, transform: "skewX(-14deg)" }} />
      <div style={{ fontFamily: COND, fontWeight: 700, fontSize: size, letterSpacing: `${mix(p, 0.5, 0.16)}em`, color, textTransform: "uppercase", whiteSpace: "nowrap", lineHeight: 1 }}>{text}</div>
    </div>
  );
};

/** A huge outlined word drifting sideways behind the scene. */
export const Ghost: React.FC<{ text: string; y: number; size?: number; color?: string; speed?: number; opacity?: number }> = ({ text, y, size = 330, color = CREAM, speed = 90, opacity = 0.12 }) => {
  const t = useT();
  const line = `${text}  ·  `.repeat(6);
  return (
    <div style={{ position: "absolute", left: -((t * speed) % (size * text.length * 0.8)) - 100, top: y, whiteSpace: "nowrap", fontFamily: DISPLAY, fontWeight: 900, fontSize: size, lineHeight: 1, letterSpacing: "-0.03em", color: "transparent", WebkitTextStroke: `2.5px ${alpha(color, opacity)}`, textTransform: "uppercase" }}>{line}</div>
  );
};

/** Pill with a count or a fact. */
export const Chip: React.FC<{ text: string; at: number; color?: string; ink?: string; size?: number; style?: React.CSSProperties }> = ({ text, at, color = MINT, ink = INK, size = 30, style }) => {
  const t = useT();
  const s = pop(t, at);
  return (
    <div style={{ display: "inline-block", padding: `${size * 0.34}px ${size * 0.72}px`, borderRadius: size, background: color, color: ink, fontFamily: COND, fontWeight: 700, fontSize: size, letterSpacing: "0.12em", textTransform: "uppercase", whiteSpace: "nowrap", lineHeight: 1, transform: `scale(${s})`, opacity: Math.min(1, s * 2), ...style }}>{text}</div>
  );
};

// ---------------------------------------------------------------------------
// transitions and accents
// ---------------------------------------------------------------------------
/** Three skewed bars sweep in one after another; the last colour fills the
 *  frame at the cut (`at`) and leaves on its own, uncovering the next scene. */
export const Wipe: React.FC<{ at: number; colors: [string, string, string]; dir?: 1 | -1 }> = ({ at, colors, dir = 1 }) => {
  const t = useT();
  if (t < at - 0.4 || t > at + 0.4) return null;
  return (
    <AbsoluteFill style={{ overflow: "hidden" }}>
      {colors.map((c, i) => {
        const d = (i - 2) * 0.06;
        const p = interpolate(t, [at - 0.26 + d, at - 0.02 + d, at + 0.03, at + 0.32], [-1, 0, 0, 1], { ...clamp, easing: inOut });
        return <div key={i} style={{ position: "absolute", top: -60, left: -W * 0.25, width: W * 1.5, height: H + 120, background: c, transform: `translateX(${p * dir * W * 1.5}px) skewX(${-16 * dir}deg)` }} />;
      })}
    </AbsoluteFill>
  );
};

export const Flash: React.FC<{ at: number; color?: string; strength?: number; len?: number }> = ({ at, color = WHITE, strength = 0.8, len = 0.3 }) => {
  const t = useT();
  if (t < at || t > at + len) return null;
  return <AbsoluteFill style={{ background: color, opacity: strength * (1 - ramp(t, at, at + len)) }} />;
};

/** Ring pulse on a point with an elbow line to a label. Coordinates are canvas pixels. */
export const Callout: React.FC<{ at: number; out: number; x: number; y: number; lx: number; ly: number; title: string; sub?: string; accent?: string; align?: "left" | "right" }> = ({ at, out, x, y, lx, ly, title, sub, accent = MINT, align = "left" }) => {
  const t = useT();
  if (t < at || t > out + 0.3) return null;
  const o = 1 - ramp(t, out, out + 0.25, inExpo);
  const p = ramp(t, at, at + 0.45);
  const q = ramp(t, at + 0.2, at + 0.7);
  const pulse = ((t - at) * 1.1) % 1;
  const len = Math.hypot(lx - x, ly - y), ang = (Math.atan2(ly - y, lx - x) * 180) / Math.PI;
  return (
    <div style={{ position: "absolute", inset: 0, opacity: o }}>
      <div style={{ position: "absolute", left: x - 34, top: y - 34, width: 68, height: 68, borderRadius: "50%", border: `3px solid ${accent}`, transform: `scale(${p})`, boxShadow: `0 0 24px ${alpha(accent, 0.7)}` }} />
      <div style={{ position: "absolute", left: x - 34, top: y - 34, width: 68, height: 68, borderRadius: "50%", border: `2px solid ${accent}`, transform: `scale(${1 + pulse * 1.1})`, opacity: (1 - pulse) * 0.8 * p }} />
      <div style={{ position: "absolute", left: x, top: y - 1.5, width: len * q, height: 3, background: accent, transformOrigin: "0 50%", transform: `rotate(${ang}deg)` }} />
      <div style={{ position: "absolute", top: ly - 30, ...(align === "left" ? { left: lx + 16 } : { right: W - lx + 16 }), textAlign: align, opacity: q, transform: `translateX(${(1 - q) * (align === "left" ? -24 : 24)}px)` }}>
        <div style={{ fontFamily: DISPLAY, fontWeight: 600, fontSize: 30, color: CREAM, lineHeight: 1.1, whiteSpace: "nowrap" }}>{title}</div>
        {sub && <div style={{ fontFamily: COND, fontWeight: 600, fontSize: 24, letterSpacing: "0.14em", color: accent, textTransform: "uppercase", marginTop: 6, whiteSpace: "nowrap" }}>{sub}</div>}
      </div>
    </div>
  );
};

/** Thin frame corners + running code, for the "broadcast" feel. */
export const Frame: React.FC<{ accent: string; label: string; right?: string }> = ({ accent, label, right }) => {
  const t = useT();
  const L = 42, S = 3;
  const c = (st: React.CSSProperties, k: number) => <div key={k} style={{ position: "absolute", width: L, height: L, ...st }} />;
  return (
    <AbsoluteFill style={{ pointerEvents: "none" }}>
      {c({ left: 44, top: 40, borderLeft: `${S}px solid ${accent}`, borderTop: `${S}px solid ${accent}` }, 0)}
      {c({ right: 44, top: 40, borderRight: `${S}px solid ${accent}`, borderTop: `${S}px solid ${accent}` }, 1)}
      {c({ left: 44, bottom: 40, borderLeft: `${S}px solid ${accent}`, borderBottom: `${S}px solid ${accent}` }, 2)}
      {c({ right: 44, bottom: 40, borderRight: `${S}px solid ${accent}`, borderBottom: `${S}px solid ${accent}` }, 3)}
      <div style={{ position: "absolute", left: 104, top: 44, fontFamily: COND, fontWeight: 600, fontSize: 22, letterSpacing: "0.22em", color: alpha(CREAM, 0.75), textTransform: "uppercase" }}>{label}</div>
      {right && <div style={{ position: "absolute", right: 104, top: 44, fontFamily: COND, fontWeight: 600, fontSize: 22, letterSpacing: "0.22em", color: alpha(CREAM, 0.75), textTransform: "uppercase" }}>{right}</div>}
      <div style={{ position: "absolute", left: 104, bottom: 44, width: 220, height: 3, background: alpha(CREAM, 0.18) }}>
        <div style={{ width: `${((t * 31) % 100)}%`, height: "100%", background: accent }} />
      </div>
    </AbsoluteFill>
  );
};

export const Sfx: React.FC<{ at: number; name: string; vol?: number }> = ({ at, name, vol = 0.3 }) => (
  <Sequence from={F(at)} durationInFrames={F(2.2)} layout="none"><Audio src={staticFile(`sfx/${name}.mp3`)} volume={vol} /></Sequence>
);

export { AbsoluteFill, Audio, Sequence, interpolate, staticFile };
