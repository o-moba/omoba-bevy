import { identity, loadGlb, multiply } from "../viewer/glb.js";
import { circle, color, makeCamera, makeProgram, transform, upload } from "../viewer/render.js";
import { ROLES, applyControl, createMatch, setHuman, snapshot, step } from "./match.mjs";

const canvas = document.querySelector("#arena");
const markers = document.querySelector("#markers");
const overlay = markers.getContext("2d");
const show = (id, visible) => { document.getElementById(id).hidden = !visible; };
const number = (id, value) => { document.getElementById(id).textContent = value; };
const roleButtons = [...document.querySelectorAll("#roles button")];
const attackButton = document.querySelector("#attack");
const abilityButton = document.querySelector("#ability");
const restartButton = document.querySelector("#restart");
const lanMode = location.protocol !== "file:" && new URLSearchParams(location.search).get("lan") === "1";
const cameraEye = [6, 30, 29];
const cameraTarget = [0, 0, 0];
const teamColors = [
  { cloth: color(0x4167df), trim: color(0x718ef1), hud: "#3459c6", light: "#b8c8ff" },
  { cloth: color(0xdf5035), trim: color(0xf18558), hud: "#c74b32", light: "#ffba9e" },
];
const proportions = { adc: 0.96, tank: 1.13, support: 0.90 };
const roleMark = { adc: "◆", tank: "⬟", support: "+" };
const movement = ["KeyW", "KeyA", "KeyS", "KeyD", "ArrowUp", "ArrowLeft", "ArrowDown", "ArrowRight"];
const keys = new Set();
const lerp = (start, end, amount) => start + (end - start) * amount;
const clamp = (value, limit) => Math.max(-limit, Math.min(limit, value));

let match = null;
let room = null;
let view = snapshot(createMatch());
let playerId = null;
let selectedRole = null;
let camera;
let keyboardMove = false;
let pointerAttack = false;
let attackPulse = false;
let lastMove = 0;
let lastHud = 0;
let selection = 0;
let noticeTimer;

function floorHeight({ x, z }) {
  if (Math.abs(z) > 3.13) return 0;
  const ramp = Math.min(1, Math.max(0, (2.05 - Math.abs(x)) / 0.4));
  return lerp(0.035, 0.205, ramp);
}

function project(x, y, z) {
  const m = camera.viewProjection;
  return [
    (1 + m[0] * x + m[4] * y + m[8] * z + m[12]) * canvas.clientWidth / 2,
    (1 - m[1] * x - m[5] * y - m[9] * z - m[13]) * canvas.clientHeight / 2,
  ];
}

function pointOnLane(px, py) {
  const x = (px * 2 / canvas.clientWidth - 1) * camera.halfWidth;
  const y = (1 - py * 2 / canvas.clientHeight) * camera.halfHeight;
  const [dx, dy, dz] = cameraTarget.map(
    (component, axis) => component + x * camera.right[axis] + y * camera.up[axis],
  );
  const depth = (dy - 0.035) / camera.back[1];
  return [clamp(dx - depth * camera.back[0], 24), clamp(dz - depth * camera.back[2], 2.8)];
}

function me() {
  return view.heroes.find(hero => hero.id === playerId);
}

function send(control) {
  if (!playerId) return;
  if (room) room.send(control);
  else if (match) applyControl(match, playerId, control);
}

function announce(message) {
  clearTimeout(noticeTimer);
  number("notice", message);
  show("notice", Boolean(message));
  if (message) noticeTimer = setTimeout(() => show("notice", false), 3500);
}

function receive(next) {
  view = next;
}

function refreshHud() {
  const hero = me();
  if (!hero) return;
  const config = ROLES[hero.role];
  const elapsed = Math.floor(view.time);
  number("blue-score", view.score[0]);
  number("orange-score", view.score[1]);
  number("time", `${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, "0")}`);
  number("health", hero.hp > 0
    ? `${config.label} · ${Math.ceil(hero.hp)} / ${hero.maxHp} HP`
    : `Возрождение через ${Math.ceil(hero.respawn)} с`);
  const targetNexus = view.structures.find(structure => structure.kind === "nexus" && structure.team !== hero.team);
  number("objective", targetNexus?.protected ? "Цель · вражеская башня" : "Цель · вражеский нексус");
  const cooldown = Math.ceil(hero.cooldown);
  number("skill", cooldown > 0 ? `${config.skill} · ${cooldown} с` : config.skill);
  abilityButton.disabled = cooldown > 0 || hero.hp <= 0 || view.winner !== null;
  attackButton.disabled = hero.hp <= 0 || view.winner !== null;
  const ended = view.winner !== null;
  show("end", ended);
  if (ended) {
    number("outcome", view.winner === hero.team ? "Победа" : "Поражение");
    number("result", view.winner === 0 ? "Синие разрушили вражеский нексус." : "Коралловые разрушили вражеский нексус.");
  }
}

function leave(message) {
  ++selection;
  room?.close();
  room = null;
  match = null;
  playerId = null;
  keyboardMove = false;
  pointerAttack = false;
  attackPulse = false;
  keys.clear();
  receive(snapshot(createMatch()));
  show("lobby", true);
  show("score", false);
  show("hud", false);
  show("leave", false);
  show("end", false);
  for (const button of roleButtons) button.disabled = false;
  number("status", message || (lanMode ? "Общая комната · свободные роли занимают боты" : "Одиночный матч · остальные роли занимают боты"));
}

async function play(role) {
  for (const button of roleButtons) button.disabled = true;
  selectedRole = role;
  if (lanMode) {
    const request = ++selection;
    try {
      number("status", "Подключаюсь к комнате…");
      const { connectLan } = await import("./network.mjs");
      const connection = await connectLan(role, {
        onSnapshot: next => { if (selection === request) receive(next); },
        onDisconnect: error => { if (selection === request) leave(error.message); },
      });
      if (selection !== request) {
        connection.close();
        return;
      }
      room = connection;
      playerId = connection.playerId;
      receive(connection.snapshot);
    } catch (error) {
      if (selection === request) leave(error.message);
      return;
    }
  } else {
    match = createMatch();
    playerId = `0-${role}`;
    setHuman(match, playerId);
    receive(snapshot(match));
  }
  show("lobby", false);
  show("hud", true);
  show("score", true);
  show("leave", true);
  refreshHud();
  announce("WASD, стрелки или клик по дорожке. Клик по врагу — атаковать.");
}

function updateInput(now) {
  if (!playerId || view.winner !== null) return;
  const horizontal = Number(keys.has("KeyD") || keys.has("ArrowRight")) -
    Number(keys.has("KeyA") || keys.has("ArrowLeft"));
  const vertical = Number(keys.has("KeyS") || keys.has("ArrowDown")) -
    Number(keys.has("KeyW") || keys.has("ArrowUp"));
  const hero = me();
  if ((horizontal || vertical) && hero && now - lastMove > 65) {
    const [rx, , rz] = camera.right;
    send({ move: [
      clamp(hero.x + (horizontal * rx - vertical * rz) * 2.5, 24),
      clamp(hero.z + (horizontal * rz + vertical * rx) * 2.5, 2.8),
    ] });
    keyboardMove = true;
    lastMove = now;
  } else if (!horizontal && !vertical && keyboardMove) {
    send({ move: null });
    keyboardMove = false;
  }
}

function selectPoint(event) {
  const hero = me();
  if (!hero || hero.hp <= 0 || view.winner !== null || event.button !== 0) return;
  const bounds = canvas.getBoundingClientRect();
  const px = event.clientX - bounds.left;
  const py = event.clientY - bounds.top;
  const towerRadius = Math.max(25, canvas.clientHeight / (2 * camera.halfHeight) * 1.8);
  let selected = null;
  let best = Infinity;
  for (const unit of [...view.heroes, ...view.minions, ...view.structures]) {
    if (unit.team === hero.team || unit.hp <= 0 || unit.protected) continue;
    const y = unit.kind === "minion" ? 0.38 : unit.kind === "tower" ? 1.76 : 1.10;
    const point = project(unit.x, floorHeight(unit) + y, unit.z);
    const distance = Math.hypot(px - point[0], py - point[1]);
    const radius = unit.kind === "minion" ? 13 : unit.kind === "tower" ? towerRadius : 20;
    if (distance < radius && distance < best) {
      best = distance;
      selected = unit;
    }
  }
  keyboardMove = false;
  if (selected) send({ target: selected.id, move: null });
  else send({ move: pointOnLane(px, py), target: null });
}

function updateAttack() {
  const held = pointerAttack || keys.has("Space");
  if (held && match) attackPulse = true;
  send({ attack: held });
}

function controls() {
  for (const button of roleButtons) button.addEventListener("click", () => play(button.dataset.role));
  document.querySelector("#leave").addEventListener("click", () => leave());
  restartButton.addEventListener("click", async () => {
    if (!room) {
      await play(selectedRole);
      return;
    }
    restartButton.disabled = true;
    try {
      receive(await room.restart());
      refreshHud();
    } catch (error) {
      announce(error.message);
    } finally {
      restartButton.disabled = false;
    }
  });
  abilityButton.addEventListener("click", () => send({ ability: true }));
  attackButton.addEventListener("pointerdown", event => {
    attackButton.setPointerCapture(event.pointerId);
    pointerAttack = true;
    updateAttack();
  });
  for (const type of ["pointerup", "pointercancel", "lostpointercapture"]) {
    attackButton.addEventListener(type, () => { pointerAttack = false; updateAttack(); });
  }
  attackButton.addEventListener("click", event => {
    if (event.detail !== 0) return;
    if (match) attackPulse = true;
    send({ attack: true });
    send({ attack: false });
  });
  window.addEventListener("keydown", event => {
    if (!playerId || event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.code === "KeyQ") {
      if (!event.repeat) send({ ability: true });
    } else if (event.code === "Space") {
      if (!event.repeat) { keys.add("Space"); updateAttack(); }
    } else if (movement.includes(event.code)) keys.add(event.code);
    else return;
    event.preventDefault();
  });
  window.addEventListener("keyup", event => {
    keys.delete(event.code);
    if (event.code === "Space") updateAttack();
  });
  window.addEventListener("blur", () => {
    keys.clear();
    pointerAttack = false;
    updateAttack();
  });
  canvas.addEventListener("pointerdown", selectPoint);
}

function structureId(nodeName) {
  const parts = /^(Blue|Orange) (Tower|Nexus)\b/.exec(nodeName);
  return parts ? `${parts[1] === "Blue" ? 0 : 1}-${parts[2].toLowerCase()}` : null;
}

function makeMinion(gl) {
  const positions = [];
  const normals = [];
  const sides = [[0.31, 0.24, 0], [0, 0.24, 0.31], [-0.31, 0.24, 0], [0, 0.24, -0.31]];
  for (let index = 0; index < sides.length; index++) {
    const before = sides[index], after = sides[(index + 1) % sides.length];
    for (const tip of [[0, 0.59, 0], [0, 0.015, 0]]) {
      const [x, y, z] = before.map((value, axis) => value + after[axis]);
      const normal = [x, tip[1] > y / 2 ? 0.3 : -0.3, z];
      positions.push(...before, ...after, ...tip);
      for (let vertex = 0; vertex < 3; vertex++) normals.push(...normal);
    }
  }
  return upload(gl, { matrix: identity, positions: Float32Array.from(positions), normals: Float32Array.from(normals) });
}

function markerBar(unit, px, py, width, height = 4) {
  const { hud, light } = teamColors[unit.team];
  overlay.fillStyle = "#1f2b20";
  overlay.fillRect(px - width / 2 - 1, py - 1, width + 2, height + 2);
  overlay.fillStyle = "#e3e5d9";
  overlay.fillRect(px - width / 2, py, width, height);
  overlay.fillStyle = unit.protected ? light : hud;
  overlay.fillRect(px - width / 2, py, width * Math.min(1, unit.hp / unit.maxHp), height);
}

function drawMarkers(heroTransforms, pixelRatio) {
  overlay.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0);
  overlay.clearRect(0, 0, canvas.clientWidth, canvas.clientHeight);
  if (!playerId) return;

  for (const effect of view.effects) {
    const [x, y] = project(effect.x, floorHeight(effect) + 0.85, effect.z);
    const target = effect.toX === undefined ? effect : { x: effect.toX, z: effect.toZ };
    const [tx, ty] = project(target.x, floorHeight(target) + 0.70, target.z);
    overlay.globalAlpha = Math.min(1, effect.remaining * 8);
    overlay.strokeStyle = effect.type === "heal" ? "#faffdc" : teamColors[effect.team].hud;
    overlay.lineWidth = effect.type === "attack" ? 1.6 : 3.2;
    overlay.beginPath();
    if (Math.hypot(x - tx, y - ty) < 4) overlay.arc(tx, ty, 9 + (1 - overlay.globalAlpha) * 7, 0, 2 * Math.PI);
    else {
      overlay.moveTo(x, y);
      overlay.lineTo(tx, ty);
    }
    overlay.stroke();
  }
  overlay.globalAlpha = 1;
  for (const structure of view.structures) {
    if (structure.hp <= 0) continue;
    const top = structure.kind === "tower" ? 4.13 : 2.9;
    const [px, py] = project(structure.x, floorHeight(structure) + top, structure.z);
    markerBar(structure, px, py, 40);
    if (structure.protected) {
      overlay.fillStyle = "#27332b";
      overlay.textAlign = "center";
      overlay.font = "600 9px system-ui, sans-serif";
      overlay.fillText("◆", px, py - 4);
    }
  }
  for (const minion of view.minions) {
    if (minion.hp >= minion.maxHp || minion.hp <= 0) continue;
    const point = project(minion.x, floorHeight(minion) + 0.78, minion.z);
    markerBar(minion, ...point, 11, 2);
  }
  for (const hero of view.heroes) {
    if (hero.hp <= 0) continue;
    const visual = heroTransforms.get(hero.id);
    const [px, py] = project(visual.x, floorHeight(visual) + 2.69 * proportions[hero.role], visual.z);
    markerBar(hero, px, py, hero.id === playerId ? 32 : 23);
    overlay.fillStyle = "#27332b";
    overlay.textAlign = "center";
    overlay.font = "800 11px system-ui, sans-serif";
    overlay.fillText(roleMark[hero.role], px, py - 3);
  }
}

async function main() {
  const gl = canvas.getContext("webgl2", { antialias: true, alpha: false });
  if (!gl || !overlay) throw new Error("WebGL 2 is unavailable");
  const [arena, character] = await Promise.all([
    loadGlb("../aram-arena.glb", "aram-arena-model"),
    loadGlb("../character-preview.glb", "aram-character-model"),
  ]);
  const arenaMeshes = arena.map(primitive => ({
    ...upload(gl, primitive),
    structure: structureId(primitive.nodeName),
  }));
  const characterMeshes = character.map(primitive => upload(gl, primitive));
  const ring = circle(gl, 0.86);
  const disk = circle(gl, 0);
  const minionMesh = makeMinion(gl);
  const shadow = color(0x2a3126, 0.19);
  const program = makeProgram(gl);
  const uniforms = Object.fromEntries(["viewProjection", "modelMatrix", "baseColor", "lighting"]
    .map(name => [name, gl.getUniformLocation(program, name)]));
  gl.useProgram(program);
  gl.enable(gl.DEPTH_TEST);
  gl.enable(gl.BLEND);
  gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  gl.clearColor(228 / 255, 235 / 255, 226 / 255, 1);

  function draw(mesh, matrix, fill, lit) {
    gl.bindVertexArray(mesh.vao);
    gl.uniformMatrix4fv(uniforms.modelMatrix, false, matrix);
    gl.uniform4fv(uniforms.baseColor, fill);
    gl.uniform1f(uniforms.lighting, lit);
    if (mesh.indexed) gl.drawElements(gl.TRIANGLES, mesh.count, gl.UNSIGNED_INT, 0);
    else gl.drawArrays(gl.TRIANGLES, 0, mesh.count);
  }

  const heroTransforms = new Map();
  let previous = performance.now();
  let previousGameTime = 0;

  function frame(now) {
    const dt = Math.min((now - previous) / 1000, 0.05);
    previous = now;
    const pixelRatio = Math.min(window.devicePixelRatio, 2);
    const width = Math.max(1, Math.round(canvas.clientWidth * pixelRatio));
    const height = Math.max(1, Math.round(canvas.clientHeight * pixelRatio));
    if (!camera || width !== canvas.width || height !== canvas.height) {
      canvas.width = markers.width = width;
      canvas.height = markers.height = height;
      gl.viewport(0, 0, width, height);
      camera = makeCamera(width / height, cameraEye, cameraTarget, 27.3, 12.0);
      gl.uniformMatrix4fv(uniforms.viewProjection, false, camera.viewProjection);
    }

    updateInput(now);
    if (match) {
      if (attackPulse) applyControl(match, playerId, { attack: true });
      step(match, dt);
      if (attackPulse && !pointerAttack && !keys.has("Space")) {
        applyControl(match, playerId, { attack: false });
      }
      attackPulse = false;
      receive(snapshot(match));
    }
    if (view.time < previousGameTime) heroTransforms.clear();
    previousGameTime = view.time;
    if (now - lastHud > 90) {
      refreshHud();
      lastHud = now;
    }

    const destroyed = new Set(view.structures.filter(unit => unit.hp <= 0).map(unit => unit.id));
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    for (const mesh of arenaMeshes) {
      if (!destroyed.has(mesh.structure)) draw(mesh, mesh.matrix, mesh.color, 1);
    }

    gl.depthMask(false);
    for (const unit of [...view.minions, ...view.heroes]) {
      if (unit.hp <= 0) {
        heroTransforms.delete(unit.id);
        continue;
      }
      const floor = floorHeight(unit);
      if (unit.kind === "minion") {
        draw(disk, transform(unit.x, floor + 0.012, unit.z, unit.yaw, 0.30, 0.30), shadow, 0);
        continue;
      }

      const visual = heroTransforms.get(unit.id) ?? { x: unit.x, z: unit.z, yaw: unit.yaw ?? 0, bob: 0 };
      const gap = Math.hypot(unit.x - visual.x, unit.z - visual.z);
      const amount = gap > 3 ? 1 : Math.min(1, dt * 18);
      visual.x = lerp(visual.x, unit.x, amount);
      visual.z = lerp(visual.z, unit.z, amount);
      const turn = (unit.yaw ?? 0) - visual.yaw;
      visual.yaw += Math.atan2(Math.sin(turn), Math.cos(turn)) * amount;
      visual.bob = lerp(visual.bob, gap > 0.025 ? 0.075 : 0, Math.min(1, dt * 12));
      heroTransforms.set(unit.id, visual);
      const radius = proportions[unit.role];
      const base = floorHeight(visual);
      draw(disk, transform(visual.x, base + 0.012, visual.z - 0.05, visual.yaw, 0.72 * radius, 0.40 * radius), shadow, 0);
      if (unit.id === playerId) {
        draw(ring, transform(visual.x, base + 0.020, visual.z, 0, radius, radius), teamColors[unit.team].cloth, 0);
      }
    }
    gl.depthMask(true);
    for (const minion of view.minions) {
      if (minion.hp <= 0) continue;
      draw(minionMesh, transform(minion.x, floorHeight(minion), minion.z, minion.yaw),
        teamColors[minion.team].cloth, 1);
    }

    for (const hero of view.heroes) {
      if (hero.hp <= 0) continue;
      const model = heroTransforms.get(hero.id);
      const radius = proportions[hero.role];
      const bounce = model.bob * Math.abs(Math.sin(now / 75));
      const pose = transform(model.x, floorHeight(model) + bounce, model.z, model.yaw, radius, radius, radius);
      for (const mesh of characterMeshes) {
        const palette = teamColors[hero.team];
        const fill = mesh.material === "Team Cloth" ? palette.cloth : mesh.material === "Team Trim" ? palette.trim : mesh.color;
        draw(mesh, multiply(pose, mesh.matrix), fill, 1);
      }
    }
    drawMarkers(heroTransforms, pixelRatio);
    requestAnimationFrame(frame);
  }

  if (lanMode) number("lobby-caption", "До трёх человек в синей команде против ботов. Снесите башню, затем нексус.");
  controls();
  leave();
  requestAnimationFrame(frame);
}

main().catch(error => {
  number("status", "Арена не загрузилась. Проверьте поддержку WebGL 2 в браузере.");
  for (const button of roleButtons) button.disabled = true;
  console.error(error);
});
