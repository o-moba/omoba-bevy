// SPDX-License-Identifier: MPL-2.0
const pollInterval = 67;
const disconnectAfter = 6500;

async function roomRequest(action, body) {
  let response;
  try {
    response = await fetch(`/api/${action}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
      keepalive: action === "leave",
      signal: AbortSignal.timeout(1700),
    });
  } catch {
    throw new Error("Связь с комнатой прервалась. Проверьте подключение к серверу.");
  }
  if (response.status === 204) return;
  const data = await response.json().catch(() => ({}));
  if (!response.ok || !data.snapshot) {
    const error = new Error(data.error ?? "Сервер комнаты недоступен.");
    error.status = response.status;
    throw error;
  }
  return data;
}

export async function connectLan(role, { onSnapshot = () => {}, onDisconnect = () => {} } = {}) {
  const joined = await roomRequest("join", { role });
  const { token, playerId } = joined;
  let shownRevision = joined.revision;
  let lastReply = performance.now();
  let sequence = 0;
  let pending = {};
  let inFlight = null;
  let attackHeld = false;
  let attackPressed = false;
  let sentAttack = false;
  let timer;
  let closed = false;

  function publish(update) {
    if (!closed && update.revision > shownRevision) {
      shownRevision = update.revision;
      onSnapshot(update.snapshot);
    }
  }

  function close() {
    if (closed) return;
    closed = true;
    clearTimeout(timer);
    window.removeEventListener("pagehide", close);
    void roomRequest("leave", { token }).catch(() => {});
  }

  function send(control) {
    if (closed) return;
    const { attack, ...patch } = control;
    if (typeof attack === "boolean") {
      attackPressed ||= attack && !attackHeld;
      attackHeld = attack;
    }
    if (patch.move && !Object.hasOwn(patch, "target")) pending.target = null;
    const queuedAbility = pending.ability === true || patch.ability === true;
    Object.assign(pending, patch);
    if (queuedAbility) pending.ability = true;
  }

  async function poll() {
    if (closed) return;
    if (!inFlight) {
      // Give quick taps one authoritative tick even when keydown and keyup share a poll.
      const attack = attackHeld || attackPressed;
      if (attack !== sentAttack) pending.attack = attack;
      sentAttack = attack;
      attackPressed = false;
      inFlight = { token, sequence: ++sequence, control: pending };
      pending = {};
    }
    let update;
    try {
      update = await roomRequest("play", inFlight);
    } catch (error) {
      if (closed) return;
      if (error.status === 401 || error.status === 400 || performance.now() - lastReply > disconnectAfter) {
        close();
        onDisconnect(error);
      } else {
        timer = setTimeout(poll, 250);
      }
      return;
    }
    if (closed) return;
    inFlight = null;
    lastReply = performance.now();
    timer = setTimeout(poll, pollInterval);
    publish(update);
  }

  async function restart() {
    if (closed) throw new Error("Выберите роль, чтобы вернуться в комнату.");
    const update = await roomRequest("restart", { token });
    publish(update);
    return update.snapshot;
  }

  window.addEventListener("pagehide", close);
  timer = setTimeout(poll, pollInterval);
  return { playerId, snapshot: joined.snapshot, send, close, restart };
}
