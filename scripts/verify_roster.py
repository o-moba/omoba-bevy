#!/usr/bin/env python3
"""Loopback two-peer smoke: all nine presets and 36 ordinary skill admissions.
Uses the existing sandbox only to arrange actors/reset cooldowns. This does not
claim competitive balance, manual input or exhaustive skill outcome verification.
"""
import json
import verify_standard_kits as base

ROSTER=['cinderforge','edgeweaver','stormfist','veilstalker','emberveil','orbitwright','riftshot','chainkeeper','frostguard']

def run(proof):
    caster, enemy=proof.peers
    caster.hero=ROSTER[0];enemy.hero='warrior'
    for peer in proof.peers:
        peer.join();proof.wait(lambda p=peer:p.player() is not None,peer.hero+' join',8)
    base.configure(proof,enemy,[6.0,0.0])
    for hero in ROSTER:
        caster.hero=hero;base.configure(proof,caster,[0.0,0.0])
        base.require(caster.player()['loadout']['recipe']['core']==hero,'Wrong resolved core')
        proof.passed(hero+'_preset_recipe')
        for slot in range(4):
            proof.sandbox(caster,dict(action='reset_actor',actor='player'))
            proof.sandbox(enemy,dict(action='reset_actor',actor='player'))
            proof.pump(.15)
            before=caster.player()['action_sequence']
            if (hero,slot) in [('stormfist',1),('orbitwright',2),('frostguard',1)]:aim=[0.0,0.0]
            elif (hero,slot) in [('edgeweaver',0),('stormfist',3),('veilstalker',2)]:
                # Targeted melee skills need a genuinely nearby enemy, not an over-range aim.
                proof.sandbox(enemy,dict(action='teleport',actor='player',position=[3.0,0.0]));aim=[3.0,0.0]
            else:aim=[6.0,0.0]
            packet=caster.cast(slot,aim)
            proof.wait(lambda:caster.player()['action_sequence']>before,f'{hero} slot {slot} action accepted')
            base.require(caster.player()['loadout']['cast_request_id']==packet['request_id'],'Missing processed cast acknowledgement')
            proof.passed(f'{hero}_{"QWER"[slot]}_ordinary_cast',tick=caster.last['snapshot_tick'],request=packet['request_id'],mana=caster.player()['mana'])
            replay_before=caster.player()['action_sequence'];caster.send(packet);proof.pump(.12)
            base.require(caster.player()['action_sequence']==replay_before,'Replay executed twice')
            proof.sandbox(enemy,dict(action='teleport',actor='player',position=[6.0,0.0]))
    proof.passed('all_36_replays_rejected')

if __name__=='__main__':
    base.run=run
    raise SystemExit(base.main())
