// SPDX-License-Identifier: MPL-2.0
// The browser and the local room server run the same simulation.
export const ROLE_IDS = Object.freeze(["adc", "tank", "support"]);
export const ROLES = Object.freeze({
  adc: Object.freeze({
    label: "Стрелок", skill: "Залп", maxHp: 125, speed: 5.1,
    range: 5.3, attackDelay: 0.57, damage: 24, cooldown: 4.8,
  }),
  tank: Object.freeze({
    label: "Танк", skill: "Удар и щит", maxHp: 235, speed: 4.45,
    range: 1.7, attackDelay: 0.78, damage: 23, cooldown: 6.5,
  }),
  support: Object.freeze({
    label: "Саппорт", skill: "Лечение", maxHp: 150, speed: 4.8,
    range: 4.15, attackDelay: 0.74, damage: 17, cooldown: 6.3,
  }),
});

export const LANE = Object.freeze({ minX: -23.35, maxX: 23.35, minZ: -2.8, maxZ: 2.8 });
const HERO_RESPAWN = 5;
const SIMULATION_STEP = 0.05;
const WAVE_DELAY = 12;
const ROBOT_LANES = [1.4, 0, -1.4];
const STRUCTURE_RULES = {
  tower: { x: 11, maxHp: 325, range: 5.8, attackDelay: 0.95, damage: 25 },
  nexus: { x: 22, maxHp: 360 },
};
const MINION = { maxHp: 52, range: 1.05, attackDelay: 0.93, damage: 9, speed: 2.7 };

const bound = (value, min, max) => Math.min(max, Math.max(min, value));
const direction = team => team === 0 ? 1 : -1;
const distance = (a, b) => Math.hypot(a.x - b.x, a.z - b.z);
const alive = entity => entity.hp > 0;
const attackable = entity => alive(entity) && !entity.protected;
const freshControl = () => ({ move: null, target: null, attack: false, ability: false });

function unit(id, kind, team, x, z, maxHp) {
  return { id, kind, team, x, z, yaw: direction(team) * Math.PI / 2, hp: maxHp, maxHp, attackAt: 0 };
}

function spawnHero(team, role, index) {
  const spawnX = -20.1 * direction(team);
  const spawnZ = ROBOT_LANES[index] * direction(team);
  return {
    ...unit(`${team}-${role}`, "hero", team, spawnX, spawnZ, ROLES[role].maxHp),
    role, spawnX, spawnZ, human: false, abilityAt: 0, respawnAt: 0, control: freshControl(),
    shield: 0, shieldUntil: 0,
  };
}

export function createMatch() {
  const heroes = [];
  const structures = [];
  for (const team of [0, 1]) {
    heroes.push(...ROLE_IDS.map((role, index) => spawnHero(team, role, index)));
    for (const [kind, rule] of Object.entries(STRUCTURE_RULES)) {
      structures.push({
        ...unit(`${team}-${kind}`, kind, team, -rule.x * direction(team), 0, rule.maxHp),
        protected: kind === "nexus",
      });
    }
  }
  return { time: 0, accumulator: 0, winner: null, heroes, structures, minions: [], effects: [],
    score: [0, 0], minionAt: 0, nextId: 0 };
}

export function setHuman(match, playerId, isHuman = true) {
  const hero = match.heroes.find(hero => hero.id === playerId);
  if (!hero) return false;
  hero.human = isHuman === true;
  hero.control = freshControl();
  return true;
}

export function resetMatch(match) {
  const players = match.heroes.filter(hero => hero.human).map(hero => hero.id);
  Object.assign(match, createMatch());
  for (const player of players) setHuman(match, player);
  return match;
}

export function applyControl(match, playerId, input) {
  const hero = match.heroes.find(hero => hero.id === playerId);
  if (!hero?.human || !alive(hero) || match.winner !== null || !input || typeof input !== "object") return false;
  const { control } = hero;
  const { move, target, attack, ability } = input;
  if (move === null) control.move = null;
  if (Array.isArray(move) && move.length === 2 && move.every(Number.isFinite)) {
    control.move = {
      x: bound(move[0], LANE.minX, LANE.maxX), z: bound(move[1], LANE.minZ, LANE.maxZ),
    };
    control.target = null;
  }
  if (target === null) control.target = null;
  if (typeof target === "string" && target.length < 40) {
    control.target = target;
    control.move = null;
  }
  if (typeof attack === "boolean") control.attack = attack;
  if (ability === true) control.ability = true;
  return true;
}

function addEffect(match, type, source, target, remaining) {
  match.effects.push({ id: ++match.nextId, type, team: source.team,
    x: source.x, z: source.z, toX: target.x, toZ: target.z, remaining });
}

function damage(match, target, amount, team) {
  if (!attackable(target)) return;
  if (target.kind === "tower" || target.kind === "nexus") {
    // Once the teams have had time to fight over the lane, pushes become decisive.
    const siege = 1 + bound((match.time - 55) / 35, 0, 2);
    amount *= siege;
  }
  const absorbed = target.shieldUntil > match.time ? Math.min(target.shield, amount) : 0;
  if (absorbed > 0) target.shield -= absorbed;
  target.hp = Math.max(0, target.hp - (amount - absorbed));
  if (alive(target)) return;
  if (target.kind === "hero") {
    target.respawnAt = match.time + Math.min(9, HERO_RESPAWN + match.time / 28);
    target.control = freshControl();
    match.score[team]++;
  }
  if (target.kind === "tower") {
    match.structures.find(structure => structure.team === target.team && structure.kind === "nexus").protected = false;
    addEffect(match, "impact", target, target, 0.6);
  }
  if (target.kind === "nexus") {
    match.winner = team;
    addEffect(match, "impact", target, target, 0.95);
  }
}

function face(entity, other) {
  if (distance(entity, other) > 0.01) entity.yaw = Math.atan2(other.x - entity.x, other.z - entity.z);
}

function advance(entity, goal, speed, seconds, stop = 0) {
  const separation = distance(entity, goal);
  if (separation - stop < 0.03) return;
  face(entity, goal);
  const ratio = Math.min(separation - stop, speed * seconds) / separation;
  entity.x = bound(entity.x + (goal.x - entity.x) * ratio, LANE.minX, LANE.maxX);
  entity.z = bound(entity.z + (goal.z - entity.z) * ratio, LANE.minZ, LANE.maxZ);
}

function nearest(source, entities, range) {
  let best = null;
  let closest = range;
  for (const candidate of entities) {
    const candidateDistance = distance(source, candidate);
    if (candidateDistance < closest) {
      best = candidate;
      closest = candidateDistance;
    }
  }
  return best;
}

function strike(match, source, target, rules, damageScale = 1) {
  if (!target || source.attackAt > match.time || distance(source, target) > rules.range) return;
  source.attackAt = match.time + rules.attackDelay;
  face(source, target);
  addEffect(match, "attack", source, target, 0.21);
  const marksmanSiege = source.role === "adc" && (target.kind === "tower" || target.kind === "nexus") ? 1.25 : 1;
  damage(match, target, rules.damage * damageScale * marksmanSiege, source.team);
}

function shield(match, hero, amount, duration) {
  hero.shield = amount;
  hero.shieldUntil = match.time + duration;
}

function cast(match, hero, enemies, aim) {
  if (hero.abilityAt > match.time) return;
  const role = ROLES[hero.role];
  if (hero.role === "adc") {
    const target = aim && distance(hero, aim) <= 6.5 ? aim : nearest(hero, enemies, 6.5);
    if (!target) return;
    face(hero, target);
    addEffect(match, "ability", hero, target, 0.37);
    damage(match, target, 58, hero.team);
  } else if (hero.role === "tank") {
    shield(match, hero, 60, 2.4);
    addEffect(match, "ability", hero, hero, 0.43);
    for (const target of enemies) {
      if (distance(hero, target) < 2.35) damage(match, target, 44, hero.team);
    }
  } else {
    const allies = match.heroes.filter(ally => ally.team === hero.team && alive(ally)
      && distance(hero, ally) < 5.3);
    for (const ally of allies) {
      ally.hp = Math.min(ally.maxHp, ally.hp + 44);
      shield(match, ally, 17, 1.8);
      addEffect(match, "heal", hero, ally, 0.42);
    }
  }
  hero.abilityAt = match.time + role.cooldown;
}

function respawn(hero, time) {
  if (!alive(hero) && hero.respawnAt <= time) {
    Object.assign(hero, { x: hero.spawnX, z: hero.spawnZ, hp: hero.maxHp, shield: 0,
      yaw: direction(hero.team) * Math.PI / 2, attackAt: time, abilityAt: time });
  }
}

function enemyUnits(match, team) {
  return [...match.heroes, ...match.minions, ...match.structures]
    .filter(candidate => candidate.team !== team && attackable(candidate));
}

function playerTurn(match, hero, enemies, role, seconds) {
  const { control } = hero;
  const focus = enemies.find(candidate => candidate.id === control.target);
  if (control.target && !focus) control.target = null;
  if (focus) advance(hero, focus, role.speed, seconds, role.range * 0.91);
  else if (control.move) advance(hero, control.move, role.speed, seconds);
  if (control.ability) {
    control.ability = false;
    cast(match, hero, enemies, focus);
  }
  const aim = focus && alive(focus) ? focus : null;
  if (aim || control.attack) strike(match, hero, aim || nearest(hero, enemies.filter(alive), role.range), role);
}

function botTurn(match, hero, enemies, role, seconds) {
  const nearbyUnit = nearest(hero, enemies.filter(enemy => enemy.kind !== "tower" && enemy.kind !== "nexus"), 7.1);
  const objective = nearest(hero, enemies.filter(enemy => enemy.kind === "tower" || enemy.kind === "nexus"), 9);
  const hasCover = objective && match.minions.some(minion => minion.team === hero.team
    && alive(minion) && distance(minion, objective) < 4.3);
  const enemyInMelee = nearbyUnit?.kind === "hero" && distance(hero, nearbyUnit) < 2.6;
  const target = hasCover && !enemyInMelee ? objective : (nearbyUnit || objective);
  const laneEnd = { x: direction(hero.team) * 22.3, z: hero.spawnZ };
  const needHealing = match.heroes.some(ally => ally.team === hero.team && alive(ally)
    && ally.maxHp - ally.hp >= 35 && distance(hero, ally) < 5.3);
  const shouldCast = hero.role === "support" ? needHealing
    : target && distance(hero, target) < (hero.role === "tank" ? 2.27 : 6.15);
  if (shouldCast) cast(match, hero, enemies, target);
  advance(hero, target || laneEnd, role.speed * 0.92, seconds, target ? role.range * 0.93 : 0);
  const aim = target && alive(target) ? target : nearest(hero, enemies.filter(alive), role.range);
  strike(match, hero, aim, role);
}

function nextWave(match) {
  if (match.time < match.minionAt) return;
  const wave = Math.round(match.minionAt / WAVE_DELAY);
  match.minionAt += WAVE_DELAY;
  const waveSize = wave >= 6 ? 4 : 3;
  for (const team of [0, 1]) {
    for (let row = 0; row < waveSize; row++) {
      const x = direction(team) * (-19.2 - row * 0.55);
      const z = (ROBOT_LANES[row] ?? 0) * 0.8 * direction(team);
      const health = MINION.maxHp + Math.min(8, wave) * 11;
      match.minions.push(unit(`minion-${++match.nextId}`, "minion", team, x, z, health));
    }
  }
}

function minionTurn(match, minion, seconds) {
  const enemies = enemyUnits(match, minion.team);
  const structure = nearest(minion, enemies.filter(enemy => enemy.kind === "tower" || enemy.kind === "nexus"), 4.5);
  const counterpart = nearest(minion, enemies.filter(enemy => enemy.kind === "minion"), 3.3);
  const target = structure || counterpart || nearest(minion, enemies, 1.85);
  const goal = target || { x: direction(minion.team) * 22, z: minion.z * 0.95 };
  advance(minion, goal, MINION.speed, seconds, target ? MINION.range * 0.9 : 0);
  strike(match, minion, target, MINION, structure ? 2 : 1);
}

function separate(match, seconds) {
  // Keep the six same-shaped heroes from standing inside each other at a shared target.
  const heroes = match.heroes.filter(alive);
  for (let index = 0; index < heroes.length; index++) {
    for (let next = index + 1; next < heroes.length; next++) {
      const a = heroes[index], b = heroes[next];
      const separation = distance(a, b);
      if (separation >= 0.8) continue;
      const shift = Math.min((0.8 - separation) * 0.5, seconds * 2);
      const nx = separation < 0.01 ? 0 : (a.x - b.x) / separation;
      const nz = separation < 0.01 ? 1 : (a.z - b.z) / separation;
      a.x = bound(a.x + nx * shift, LANE.minX, LANE.maxX);
      a.z = bound(a.z + nz * shift, LANE.minZ, LANE.maxZ);
      b.x = bound(b.x - nx * shift, LANE.minX, LANE.maxX);
      b.z = bound(b.z - nz * shift, LANE.minZ, LANE.maxZ);
    }
  }
}

function tick(match, seconds) {
  for (const effect of match.effects) effect.remaining -= seconds;
  match.effects = match.effects.filter(effect => effect.remaining > 0);
  if (match.winner !== null) return;
  match.time += seconds;
  nextWave(match);

  for (const hero of match.heroes) respawn(hero, match.time);
  for (const hero of match.heroes) {
    if (match.winner !== null) return;
    if (!alive(hero)) continue;
    if (distance(hero, { x: -22 * direction(hero.team), z: 0 }) < 2.6) {
      hero.hp = Math.min(hero.maxHp, hero.hp + seconds * 38);
    }
    const enemies = enemyUnits(match, hero.team);
    const role = ROLES[hero.role];
    if (hero.human) playerTurn(match, hero, enemies, role, seconds);
    else botTurn(match, hero, enemies, role, seconds);
  }
  separate(match, seconds);

  for (const minion of match.minions) {
    if (match.winner !== null) return;
    if (alive(minion)) minionTurn(match, minion, seconds);
  }
  match.minions = match.minions.filter(alive);
  for (const tower of match.structures) {
    if (tower.kind !== "tower" || !alive(tower)) continue;
    const rules = STRUCTURE_RULES.tower;
    const enemies = enemyUnits(match, tower.team);
    const frontline = nearest(tower, enemies.filter(enemy => enemy.kind === "minion"), rules.range);
    const hero = nearest(tower, enemies.filter(enemy => enemy.kind === "hero"), rules.range);
    strike(match, tower, frontline || hero, rules);
  }
}

export function step(match, seconds) {
  if (!Number.isFinite(seconds) || seconds <= 0) return;
  match.accumulator += Math.min(seconds, 1);
  const ticks = Math.floor((match.accumulator + 1e-10) / SIMULATION_STEP);
  match.accumulator = Math.max(0, match.accumulator - ticks * SIMULATION_STEP);
  for (let tickIndex = 0; tickIndex < ticks; tickIndex++) tick(match, SIMULATION_STEP);
}

function publicUnit({ id, team, kind, x, z, yaw, hp, maxHp }) {
  return { id, team, kind, x, z, yaw, hp, maxHp };
}

export function snapshot(match) {
  return {
    time: match.time,
    winner: match.winner,
    heroes: match.heroes.map(hero => ({
      ...publicUnit(hero), role: hero.role, human: hero.human,
      respawn: alive(hero) ? 0 : Math.max(0, hero.respawnAt - match.time),
      cooldown: Math.max(0, hero.abilityAt - match.time),
      shield: hero.shieldUntil > match.time && alive(hero) ? hero.shield : 0,
    })),
    structures: match.structures.map(structure => ({ ...publicUnit(structure), protected: structure.protected })),
    minions: match.minions.map(publicUnit),
    effects: match.effects.map(effect => ({ ...effect })),
    score: [...match.score],
  };
}
