// SPDX-License-Identifier: MPL-2.0
import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { networkInterfaces } from "node:os";
import { parseArgs } from "node:util";
import { applyControl, createMatch, resetMatch, ROLE_IDS, setHuman, snapshot, step } from "./match.mjs";

const { values: options } = parseArgs({
  options: {
    host: { type: "string", default: "127.0.0.1" },
    port: { type: "string", default: "8080" },
  },
});
const port = Number(options.port);
if (!/^\d+$/.test(options.port) || !Number.isInteger(port) || port > 65535) {
  throw new Error("--port должен быть целым числом от 0 до 65535.");
}

const root = new URL("../", import.meta.url);
const match = createMatch();
const sessions = new Map();
const seatDuration = 8000;
let revision = 0;
let previousTick = performance.now();

function release(token, session) {
  sessions.delete(token);
  setHuman(match, session.playerId, false);
}

function expireSeats(now) {
  for (const [token, session] of sessions) {
    if (now >= session.expiresAt) release(token, session);
  }
}

const timer = setInterval(() => {
  const now = performance.now();
  const seconds = Math.min((now - previousTick) / 1000, 0.1);
  previousTick = now;
  expireSeats(now);
  if (sessions.size > 0) step(match, seconds);
}, 1000 / 30);

class RequestError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

function sendJson(response, status, data) {
  response.writeHead(status, { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store" });
  response.end(JSON.stringify(data));
}

async function readJson(request) {
  if (request.headers["content-type"]?.split(";")[0] !== "application/json") {
    throw new RequestError(415, "Комната принимает запросы в формате JSON.");
  }
  const chunks = [];
  let size = 0;
  for await (const chunk of request.iterator({ destroyOnReturn: false })) {
    size += chunk.length;
    if (size > 8192) {
      request.resume();
      throw new RequestError(413, "Слишком большой запрос к комнате.");
    }
    chunks.push(chunk);
  }
  try {
    const value = JSON.parse(Buffer.concat(chunks));
    if (value && typeof value === "object" && !Array.isArray(value)) return value;
  } catch {
    throw new RequestError(400, "Некорректный запрос к комнате.");
  }
  throw new RequestError(400, "Некорректный запрос к комнате.");
}

function readControl(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new RequestError(400, "Некорректная команда игрока.");
  }
  const control = {};
  if (Object.hasOwn(value, "move")) {
    const move = value.move;
    if (move !== null && !(Array.isArray(move) && move.length === 2 && move.every(Number.isFinite))) {
      throw new RequestError(400, "Некорректная точка перемещения.");
    }
    control.move = move;
  }
  if (Object.hasOwn(value, "target")) {
    if (value.target !== null && (typeof value.target !== "string" || value.target.length >= 40)) {
      throw new RequestError(400, "Некорректная цель.");
    }
    control.target = value.target;
  }
  for (const key of ["attack", "ability"]) {
    if (Object.hasOwn(value, key)) {
      if (typeof value[key] !== "boolean") throw new RequestError(400, "Некорректное действие.");
      control[key] = value[key];
    }
  }
  return control;
}

function state() {
  return { revision: ++revision, snapshot: snapshot(match) };
}

async function handleApi(request, response, path) {
  if (!["/api/join", "/api/play", "/api/leave", "/api/restart"].includes(path)) {
    throw new RequestError(404, "Команда комнаты не найдена.");
  }
  if (request.method !== "POST") throw new RequestError(405, "Комната ожидает POST-запрос.");
  const body = await readJson(request);
  const now = performance.now();
  expireSeats(now);

  if (path === "/api/join") {
    if (!ROLE_IDS.includes(body.role)) throw new RequestError(400, "Выберите одну из трёх ролей.");
    const playerId = `0-${body.role}`;
    if ([...sessions.values()].some(session => session.playerId === playerId)) {
      throw new RequestError(409, "Эта роль уже занята. Выберите другую.");
    }
    if (sessions.size === 0) resetMatch(match);
    const token = randomUUID();
    sessions.set(token, { playerId, expiresAt: now + seatDuration, sequence: 0 });
    setHuman(match, playerId, true);
    sendJson(response, 200, { token, playerId, ...state() });
    return;
  }

  const session = sessions.get(body.token);
  if (!session) throw new RequestError(401, "Место в комнате освободилось. Выберите роль заново.");
  if (path === "/api/leave") {
    release(body.token, session);
    response.writeHead(204).end();
    return;
  }
  if (path === "/api/restart") {
    if (match.winner !== null) resetMatch(match);
  } else {
    if (!Number.isSafeInteger(body.sequence) || body.sequence < 1) {
      throw new RequestError(400, "Некорректный номер команды.");
    }
    if (body.sequence > session.sequence) {
      applyControl(match, session.playerId, readControl(body.control));
      session.sequence = body.sequence;
    }
  }
  session.expiresAt = now + seatDuration;
  sendJson(response, 200, state());
}

const contentTypes = {
  css: "text/css; charset=utf-8",
  glb: "model/gltf-binary",
  html: "text/html; charset=utf-8",
  js: "text/javascript; charset=utf-8",
  mjs: "text/javascript; charset=utf-8",
};

function isPublicAsset(path) {
  return /^\/[a-z0-9-]+\.glb$/.test(path) ||
    /^\/viewer\/(glb|render)\.js$/.test(path) ||
    /^\/aram\/(?!server\.mjs$)[a-z0-9-]+\.(css|html|mjs)$/.test(path);
}

async function handleRequest(request, response) {
  const path = new URL(request.url, "http://localhost").pathname;
  if (path.startsWith("/api/")) return handleApi(request, response, path);
  if (request.method !== "GET" && request.method !== "HEAD") throw new RequestError(405, "Метод недоступен.");
  if (path === "/" || path === "/aram/") {
    response.writeHead(302, { Location: "/aram/index.html?lan=1" }).end();
    return;
  }
  if (!isPublicAsset(path)) throw new RequestError(404, "Файл не найден.");
  const contents = await readFile(new URL(path.slice(1), root));
  response.writeHead(200, {
    "Cache-Control": "no-cache",
    "Content-Length": contents.length,
    "Content-Type": contentTypes[path.split(".").at(-1)],
  });
  response.end(request.method === "HEAD" ? undefined : contents);
}

const server = createServer((request, response) => {
  handleRequest(request, response).catch(error => {
    if (error instanceof RequestError) sendJson(response, error.status, { error: error.message });
    else if (error.code === "ENOENT") sendJson(response, 404, { error: "Файл не найден." });
    else {
      console.error(error);
      sendJson(response, 500, { error: "Комната не смогла обработать запрос." });
    }
  });
});
server.on("close", () => clearInterval(timer));
server.listen(port, options.host, () => {
  const { address, port: boundPort } = server.address();
  if (address === "0.0.0.0") {
    const lanAddresses = Object.values(networkInterfaces()).flat()
      .filter(item => item.family === "IPv4" && !item.internal).map(item => item.address);
    for (const host of ["127.0.0.1", ...lanAddresses]) console.log(`OMOBA ARAM http://${host}:${boundPort}/`);
  } else {
    const host = address.includes(":") ? `[${address}]` : address;
    console.log(`OMOBA ARAM http://${host}:${boundPort}/`);
  }
});
