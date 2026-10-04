"""Reproduce catalog spending and conservative lane-income timing (no dependencies).

Run: python3 scripts/economy_model.py [--output report.json].
This is a pacing model, not a replacement for the Rust authority regressions.
"""
import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path)
args = parser.parse_args()
items = {item['id']: item for item in json.loads((ROOT / 'shared/assets/catalog/items.json').read_text())['items']}
heroes = json.loads((ROOT / 'shared/assets/catalog/heroes.json').read_text())['classes']
shop = (ROOT / 'shared/src/shop.rs').read_text()
balance = (ROOT / 'common/src/balance.rs').read_text()


def scalar(source, name):
    return float(re.search(rf'pub const {name}: [^=]+ = ([\d_.]+)', source).group(1).replace('_', ''))


starting = int(scalar(shop, 'STARTING_GOLD'))
passive = scalar(shop, 'GOLD_PER_SECOND')
hero_bounty = int(scalar(shop, 'HERO_KILL_GOLD'))
wave_gold = int(scalar(balance, 'MINIONS_PER_WAVE') * scalar(balance, 'MINION_KILL_GOLD'))
wave_interval = int(re.search(r'MINION_WAVE_INTERVAL: Duration = Duration::from_secs\((\d+)\)', balance).group(1))


def includes(root, target):
    return root == target or any(includes(child, target) for child in items[root].get('components', []))


def quote(item_id, inventory):
    consumed = []

    def credit(item_id):
        if item_id in consumed:
            return 0
        if item_id in inventory:
            consumed.append(item_id)
            return items[item_id]['cost']
        return sum(credit(child) for child in items[item_id].get('components', []))

    cost = items[item_id]['cost'] - sum(credit(child) for child in items[item_id].get('components', []))
    return cost, consumed


def first_affordable(cost, allies=1, kills=False, lane=True):
    # First wave is spawned at 10s; credit it only after a full minute to allow
    # travel and clearing. Further clears are one per minute. No jungle income.
    for second in range(3601):
        waves = second // wave_interval if lane else 0
        income = int(passive * second) + waves * wave_gold // allies
        income += hero_bounty if kills and second >= 120 else 0
        if starting + income >= cost:
            return second
    return None


builds = {}
for hero in heroes:
    inventory = []
    wallet = 10_080
    purchases = []
    for item_id in hero['recommended_items']:
        if any(includes(held, item_id) for held in inventory):
            continue
        cost, consumed = quote(item_id, inventory)
        if cost <= wallet and len(inventory) - len(consumed) < 6:
            wallet -= cost
            inventory = [held for held in inventory if held not in consumed] + [item_id]
            purchases.append({'id': item_id, 'charged': cost, 'consumed': consumed})
    assert 5 <= len(inventory) <= 6, (hero['id'], inventory)
    assert all(any(includes(held, item_id) for held in inventory) or len(inventory) - len(quote(item_id, inventory)[1]) >= 6 or quote(item_id, inventory)[0] > wallet for item_id in hero['recommended_items'])
    builds[hero['id']] = {'purchases': purchases, 'final_inventory': inventory, 'unspent': wallet}

milestones = {}
for item_id, item in items.items():
    milestones[item_id] = {
        'tier': item.get('tier', 1), 'total_cost': item['cost'],
        'solo_lane_seconds': first_affordable(item['cost']),
        'duo_lane_seconds': first_affordable(item['cost'], allies=2),
        'passive_only_seconds': first_affordable(item['cost'], lane=False),
        'solo_one_hero_kill_at_120s_seconds': first_affordable(item['cost'], kills=True),
    }
output = {
    'assumptions': {'starting_gold': starting, 'passive_gold_per_second': passive, 'lane_wave_gold_pool': wave_gold, 'wave_interval_seconds': wave_interval, 'first_wave_clear_seconds': 60, 'jungle_gold': 0, 'purchase_scope': 'One target item and its recipe only, no unrelated purchases. A killed or absent hero may miss waves; travel to shop adds time.'},
    'milestones': milestones,
    'respawn_seconds_by_minute': {str(minute): min(35, 5 + max(0, minute * 60 - 120) / 40) for minute in [0, 2, 5, 10, 15, 20, 22, 30]},
    'practice_recommended_plans': builds,
}
text = json.dumps(output, indent=2) + '\n'
if args.output:
    args.output.write_text(text)
    print(f'Wrote {args.output}; {len(builds)} legal build plans, {len(milestones)} item milestones.')
else:
    print(text, end='')
