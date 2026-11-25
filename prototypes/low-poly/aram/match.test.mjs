// SPDX-License-Identifier: MPL-2.0
import assert from "node:assert/strict";
import test from "node:test";
import { applyControl, createMatch, LANE, resetMatch, setHuman, snapshot, step } from "./match.mjs";

function run(match, seconds) {
  for (let elapsed = 0; elapsed < seconds; elapsed += 0.05) step(match, 0.05);
}

function controlledMatch() {
  const match = createMatch();
  for (const hero of match.heroes) setHuman(match, hero.id);
  return match;
}

test("three roles per team, move commands stay on the lane and bots own open slots", () => {
  const match = createMatch();
  assert.deepEqual(match.heroes.filter(hero => hero.team === 0).map(hero => hero.role),
    ["adc", "tank", "support"]);
  assert.equal(applyControl(match, "0-adc", { move: [0, 0] }), false);
  assert.equal(setHuman(match, "unknown"), false);
  assert.equal(setHuman(match, "0-adc"), true);

  const adc = match.heroes.find(hero => hero.id === "0-adc");
  const bot = match.heroes.find(hero => hero.id === "0-tank");
  const start = adc.x;
  applyControl(match, adc.id, { move: [-500, 500] });
  run(match, 1);
  assert.equal(adc.x, LANE.minX);
  assert.equal(adc.z, LANE.maxZ);
  assert.ok(bot.x > start);
  assert.equal(match.minions.length, 6);
});

test("tower protects nexus, destruction unlocks it and the next objective can win", () => {
  const match = controlledMatch();
  const hero = match.heroes.find(hero => hero.id === "0-adc");
  const tower = match.structures.find(structure => structure.id === "1-tower");
  const nexus = match.structures.find(structure => structure.id === "1-nexus");
  assert.equal(tower.x * 2, nexus.x);
  hero.x = nexus.x - 1;
  hero.z = 0;
  applyControl(match, hero.id, { target: nexus.id, attack: true, ability: true });
  step(match, 0.05);
  assert.equal(nexus.hp, nexus.maxHp);
  assert.equal(nexus.protected, true);

  tower.hp = 1;
  hero.x = tower.x - 2;
  hero.attackAt = 0;
  applyControl(match, hero.id, { target: tower.id });
  step(match, 0.05);
  assert.equal(tower.hp, 0);
  assert.equal(nexus.protected, false);
  assert.equal(match.winner, null);

  nexus.hp = 1;
  hero.x = nexus.x - 1;
  hero.attackAt = 0;
  applyControl(match, hero.id, { target: nexus.id });
  step(match, 0.05);
  assert.equal(snapshot(match).winner, 0);

  resetMatch(match);
  assert.equal(match.winner, null);
  assert.ok(match.heroes.every(hero => hero.human && hero.hp === hero.maxHp));
  assert.equal(match.structures.find(structure => structure.id === "1-nexus").protected, true);
});

test("tank absorbs damage, support heals team, ADC can burst and respawn follows a KO", () => {
  const match = controlledMatch();
  const [adc, tank, support, enemy] = match.heroes;
  Object.assign(adc, { x: 0, z: 1, hp: 40 });
  Object.assign(tank, { x: 0, z: 0, hp: 100 });
  Object.assign(support, { x: -1, z: -1 });
  Object.assign(enemy, { x: 1.5, z: 1, hp: 105 });

  applyControl(match, tank.id, { ability: true });
  step(match, 0.05);
  assert.ok(enemy.hp < 105);
  assert.equal(snapshot(match).heroes.find(hero => hero.id === tank.id).shield, 60);
  applyControl(match, enemy.id, { attack: true, target: tank.id });
  step(match, 0.05);
  assert.equal(tank.hp, 100);
  assert.ok(tank.shield < 60);

  applyControl(match, support.id, { ability: true });
  step(match, 0.05);
  assert.ok(tank.hp > 100 && adc.hp > 40);
  assert.ok(snapshot(match).effects.some(effect => effect.type === "heal"));

  applyControl(match, adc.id, { ability: true, attack: true, target: enemy.id });
  step(match, 0.05);
  assert.equal(enemy.hp, 0);
  assert.equal(match.score[0], 1);
  assert.ok(snapshot(match).heroes.find(hero => hero.id === enemy.id).respawn > 4);
  assert.ok(snapshot(match).heroes.find(hero => hero.id === adc.id).cooldown > 4);

  run(match, 5.3);
  assert.ok(enemy.hp > 0);
  assert.equal(snapshot(match).heroes.find(hero => hero.id === enemy.id).respawn, 0);
});

test("waves and bots settle a full match through both objectives", () => {
  const match = createMatch();
  const visited = new Set();
  for (let frame = 0; frame < 4000 && match.winner === null; frame++) {
    step(match, 0.05);
    for (const structure of match.structures) {
      if (structure.hp === 0) visited.add(structure.kind);
    }
  }
  assert.ok(match.winner === 0 || match.winner === 1);
  assert.ok(visited.has("tower") && visited.has("nexus"));
  assert.ok(match.score.some(score => score > 0));
});

test("simulation advances identically at room-server and browser refresh rates", () => {
  const server = createMatch();
  const browser = createMatch();
  for (let frame = 0; frame < 900; frame++) {
    step(server, 1 / 30);
    step(browser, 1 / 60);
    step(browser, 1 / 60);
  }
  assert.deepEqual(snapshot(server), snapshot(browser));
});
