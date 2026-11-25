import { loadGlb, multiply } from "./glb.js";
import { circle, color, makeCamera, makeProgram, transform, upload } from "./render.js";

const canvas = document.querySelector("#arena");
const status = document.querySelector("#status");
const instructions = document.querySelector("#instructions");
const cameraEye = [7, 26, 30];
const cameraTarget = [0, 0, 0];
const movementKeys = new Set(["KeyW", "KeyA", "KeyS", "KeyD", "ArrowUp", "ArrowLeft", "ArrowDown", "ArrowRight"]);
const laneHeight = 0.035;
const bridgeHeight = 0.205;

const teams = [
  {
    button: document.querySelector("#blue"),
    cloth: color(0x4167df),
    trim: color(0x718ef1),
    x: -5.5,
    z: 1.4,
    yaw: 0.45,
  },
  {
    button: document.querySelector("#ember"),
    cloth: color(0xdf5035),
    trim: color(0xf18558),
    x: 5.5,
    z: -1.2,
    yaw: -0.45,
  },
];

const clamp = (value, limit) => Math.max(-limit, Math.min(value, limit));

function floorHeight({ x, z }) {
  if (Math.abs(z) > 3.14) return 0;
  if (Math.abs(x) > 2.05) return laneHeight;
  const ramp = Math.min(1, (2.05 - Math.abs(x)) / 0.4);
  return laneHeight + (bridgeHeight - laneHeight) * ramp;
}

function pointOnFloor(event, camera) {
  const bounds = canvas.getBoundingClientRect();
  const x = ((event.clientX - bounds.left) / bounds.width * 2 - 1) * camera.halfWidth;
  const y = (1 - (event.clientY - bounds.top) / bounds.height * 2) * camera.halfHeight;
  const point = cameraTarget.map((component, axis) => component + x * camera.right[axis] + y * camera.up[axis]);
  const depth = point[1] / camera.back[1];
  return [
    clamp(point[0] - depth * camera.back[0], 15),
    clamp(point[2] - depth * camera.back[2], 8),
  ];
}

function move(team, keys, destination, right, seconds) {
  const horizontal = Number(keys.has("KeyD") || keys.has("ArrowRight")) -
    Number(keys.has("KeyA") || keys.has("ArrowLeft"));
  const vertical = Number(keys.has("KeyS") || keys.has("ArrowDown")) -
    Number(keys.has("KeyW") || keys.has("ArrowUp"));
  const keyboard = horizontal !== 0 || vertical !== 0;
  if (keyboard) destination = null;
  if (!keyboard && !destination) return null;

  const dx = destination ? destination[0] - team.x : horizontal * right[0] - vertical * right[2];
  const dz = destination ? destination[1] - team.z : horizontal * right[2] + vertical * right[0];
  const distance = Math.hypot(dx, dz);
  if (distance < 0.015) return null;
  const step = destination ? Math.min(seconds * 6.3, distance) : seconds * 6.3;
  team.x = clamp(team.x + step * dx / distance, 15);
  team.z = clamp(team.z + step * dz / distance, 8);
  team.yaw = Math.atan2(dx, dz);
  return destination;
}

async function main() {
  const gl = canvas.getContext("webgl2", { antialias: true, alpha: false });
  if (!gl) throw new Error("WebGL 2 is unavailable");
  const [arena, character] = await Promise.all([
    loadGlb("../arena.glb", "arena-model"),
    loadGlb("../character-preview.glb", "character-model"),
  ]);
  const arenaMeshes = arena.map(primitive => upload(gl, primitive));
  const characterMeshes = character.map(primitive => upload(gl, primitive));
  const footShadow = circle(gl, 0);
  const teamRing = circle(gl, 0.89);
  const shadowColor = color(0x2a3126, 0.23);
  const program = makeProgram(gl);
  const viewProjection = gl.getUniformLocation(program, "viewProjection");
  const modelMatrix = gl.getUniformLocation(program, "modelMatrix");
  const baseColor = gl.getUniformLocation(program, "baseColor");
  const lighting = gl.getUniformLocation(program, "lighting");
  gl.useProgram(program);
  gl.enable(gl.DEPTH_TEST);
  gl.enable(gl.BLEND);
  gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  gl.clearColor(228 / 255, 235 / 255, 226 / 255, 1);

  function draw(mesh, matrix, material, lit) {
    gl.bindVertexArray(mesh.vao);
    gl.uniformMatrix4fv(modelMatrix, false, matrix);
    gl.uniform4fv(baseColor, material);
    gl.uniform1f(lighting, lit);
    if (mesh.indexed) gl.drawElements(gl.TRIANGLES, mesh.count, gl.UNSIGNED_INT, 0);
    else gl.drawArrays(gl.TRIANGLES, 0, mesh.count);
  }

  let selected = teams[0];
  let destination = null;
  let camera = null;
  let previousFrame = performance.now();
  const keys = new Set();
  for (const team of teams) {
    team.button.addEventListener("click", () => {
      selected = team;
      destination = null;
      for (const candidate of teams) candidate.button.setAttribute("aria-pressed", String(candidate === selected));
    });
  }
  window.addEventListener("keydown", event => {
    if (!movementKeys.has(event.code) || event.altKey || event.ctrlKey || event.metaKey) return;
    keys.add(event.code);
    event.preventDefault();
  });
  window.addEventListener("keyup", event => keys.delete(event.code));
  window.addEventListener("blur", () => keys.clear());
  canvas.addEventListener("pointerdown", event => {
    if (event.button === 0 && camera) destination = pointOnFloor(event, camera);
  });

  function frame(now) {
    const pixelRatio = Math.min(window.devicePixelRatio, 2);
    const width = Math.max(1, Math.round(canvas.clientWidth * pixelRatio));
    const height = Math.max(1, Math.round(canvas.clientHeight * pixelRatio));
    if (!camera || width !== canvas.width || height !== canvas.height) {
      canvas.width = width;
      canvas.height = height;
      gl.viewport(0, 0, width, height);
      camera = makeCamera(width / height, cameraEye, cameraTarget);
      gl.uniformMatrix4fv(viewProjection, false, camera.viewProjection);
    }

    destination = move(selected, keys, destination, camera.right, Math.min((now - previousFrame) / 1000, 0.05));
    previousFrame = now;
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    for (const mesh of arenaMeshes) draw(mesh, mesh.matrix, mesh.color, 1);

    gl.depthMask(false);
    for (const team of teams) {
      const floor = floorHeight(team);
      draw(footShadow, transform(team.x - 0.12, floor + 0.012, team.z - 0.08, team.yaw, 0.72, 0.40), shadowColor, 0);
    }
    const marker = [...selected.cloth.slice(0, 3), 0.70];
    draw(teamRing, transform(selected.x, floorHeight(selected) + 0.015, selected.z, 0, 0.93, 0.93), marker, 0);
    gl.depthMask(true);

    for (const team of teams) {
      const instance = transform(team.x, floorHeight(team), team.z, team.yaw);
      for (const mesh of characterMeshes) {
        let material = mesh.color;
        if (mesh.material === "Team Cloth") material = team.cloth;
        if (mesh.material === "Team Trim") material = team.trim;
        draw(mesh, multiply(instance, mesh.matrix), material, 1);
      }
    }
    requestAnimationFrame(frame);
  }

  status.hidden = true;
  instructions.hidden = false;
  requestAnimationFrame(frame);
}

main().catch(error => {
  status.textContent = "Арена не загрузилась. Проверьте, что браузер поддерживает WebGL 2.";
  console.error(error);
});
