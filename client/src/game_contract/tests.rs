use super::*;
use std::path::PathBuf;

fn repo(path: &str) -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).join(path)
}

fn facts() -> Facts {
    Facts::read().expect("contract facts")
}

/// Where two JSON values differ, numbers compared by value (the wrapper writes `1` for
/// `1.0`).
fn difference(path: &str, left: &Value, right: &Value) -> Option<String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            (a.as_f64() != b.as_f64()).then(|| format!("{path}: {a} != {b}"))
        }
        (Value::Object(a), Value::Object(b)) => {
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            keys.into_iter()
                .find_map(|key| match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => difference(&format!("{path}.{key}"), x, y),
                    _ => Some(format!("{path}.{key}: present on one side only")),
                })
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(index, (x, y))| difference(&format!("{path}[{index}]"), x, y)),
        _ => (left != right).then(|| format!("{path}: {left} != {right}")),
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("JSON") + "\n"
}

/// The header of a committed document: its own keys and the provenance in its profile `ext`.
fn take_header(document: &mut Value) -> Map<String, Value> {
    let mut header = Map::new();
    let object = document.as_object_mut().expect("contract object");
    for key in HEADER_KEYS {
        if let Some(value) = object.remove(key) {
            header.insert(key.into(), value);
        }
    }
    if let Some(ext) = object
        .get_mut("ext")
        .and_then(|ext| ext.get_mut(PROFILE))
        .and_then(Value::as_object_mut)
    {
        for key in PROVENANCE_KEYS {
            if let Some(value) = ext.remove(key) {
                header.insert(format!("ext.{key}"), value);
            }
        }
    }
    header
}

/// Set `OMOBA_WRITE_GAME_CONTRACT=1` to regenerate both files, then run
/// `python3 scripts/export_game_contract.py --output shared/assets/catalog`.
#[test]
fn game_contract_is_current() {
    let facts = facts();
    let body = render_contract(&facts);
    let notes = render_asset_notes(&facts).expect("asset notes");
    let committed: Option<Value> = std::fs::read_to_string(repo(CONTRACT_PATH))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    if std::env::var_os(WRITE_VAR).is_some() {
        let mut document = body.clone();
        // Keep the wrapper's header: it recognises a changed body by its stale revision.
        if let Some(mut old) = committed {
            for (key, value) in take_header(&mut old) {
                match key.strip_prefix("ext.") {
                    Some(key) => {
                        document["ext"][PROFILE][key] = value;
                    }
                    None => {
                        document[key.as_str()] = value;
                    }
                }
            }
        }
        std::fs::write(repo(CONTRACT_PATH), pretty(&document)).unwrap();
        std::fs::write(repo(NOTES_PATH), pretty(&notes)).unwrap();
    }
    let rerun = "run OMOBA_WRITE_GAME_CONTRACT=1 cargo test -p client --lib game_contract_is_current, \
                 then python3 scripts/export_game_contract.py --output shared/assets/catalog";
    let mut committed: Value = std::fs::read_to_string(repo(CONTRACT_PATH))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| panic!("{CONTRACT_PATH} is missing; {rerun}"));
    let header = take_header(&mut committed);
    assert_eq!(
        header.len(),
        HEADER_KEYS.len() + PROVENANCE_KEYS.len(),
        "{CONTRACT_PATH} has no header; {rerun}"
    );
    if let Some(stale) = difference("contract", &committed, &body) {
        panic!("{CONTRACT_PATH} is stale ({stale}); {rerun}");
    }
    let committed_notes: Value = std::fs::read_to_string(repo(NOTES_PATH))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| panic!("{NOTES_PATH} is missing; {rerun}"));
    if let Some(stale) = difference("notes", &committed_notes, &notes) {
        panic!("{NOTES_PATH} is stale ({stale}); {rerun}");
    }
}

fn slot_list(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|slot| slot.as_str().unwrap())
        .collect()
}

#[test]
fn slots_follow_the_profile_grammar_and_targets_name_known_slots() {
    let facts = facts();
    let body = render_contract(&facts);
    let slots = body["slots"].as_object().unwrap();
    let id = |text: &str, max: usize| {
        let mut chars = text.chars();
        chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && text.len() <= max
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    for (slot, value) in slots {
        assert!(slot.len() <= 64, "{slot}");
        let parts: Vec<&str> = slot.split('.').collect();
        let valid = match parts.as_slice() {
            ["base", state] => ["idle", "run", "walk", "death"].contains(state),
            ["basic", class, index] => id(class, 32) && index.len() == 1,
            ["basic", class, mode, index] => id(class, 32) && id(mode, 16) && index.len() == 1,
            ["skill", skill, part] => {
                id(skill, 48) && ["release", "recast", "windup"].contains(part)
            }
            _ => false,
        };
        assert!(valid, "slot id {slot} breaks the profile grammar");
        assert_eq!(value["kind"], "animation");
        for link in ["sharesDefaultWith", "alternatesWith"] {
            for other in slot_list(&value[link]) {
                assert!(slots.contains_key(other), "{slot}.{link}");
                assert!(
                    slot_list(&slots[other][link]).contains(&slot.as_str()),
                    "{slot}.{link}"
                );
            }
        }
    }
    let targets = body["targets"].as_array().unwrap();
    assert_eq!(targets.len(), HeroClass::ALL.len() + 1);
    assert_eq!(targets[0]["id"], "any");
    for target in targets {
        let listed = slot_list(&target["slots"]);
        let sectioned: Vec<&str> = target["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|section| slot_list(&section["slots"]))
            .collect();
        assert_eq!(
            sectioned, listed,
            "{}: slots are the sections in order",
            target["id"]
        );
        assert_eq!(&listed[..3], ["base.idle", "base.run", "base.death"]);
        assert_eq!(listed.iter().collect::<BTreeSet<_>>().len(), listed.len());
        for slot in &listed {
            assert!(
                slots.contains_key(*slot),
                "{} names unknown {slot}",
                target["id"]
            );
        }
        let offer = &target["accepts"][0];
        assert_eq!(
            offer["requirementId"],
            format!("character.{}", target["id"].as_str().unwrap())
        );
        let custom: Vec<&str> = listed
            .iter()
            .copied()
            .filter(|slot| slots[*slot]["customizable"] == true)
            .collect();
        assert_eq!(slot_list(&offer["slots"]), custom);
    }
    // Every slot except the reserved walk belongs to at least one target.
    let viewed: BTreeSet<&str> = targets
        .iter()
        .flat_map(|target| slot_list(&target["slots"]))
        .collect();
    let unviewed: Vec<&String> = slots
        .keys()
        .filter(|slot| !viewed.contains(slot.as_str()))
        .collect();
    assert_eq!(unviewed, ["base.walk"]);
}

/// The counts the profile froze for 1.0 (CC §3.6): change them only together with the docs.
#[test]
fn slot_counts_match_the_published_contract() {
    let facts = facts();
    let count = |f: &dyn Fn(&SlotFacts) -> bool| facts.slots.values().filter(|s| f(s)).count();
    assert_eq!(facts.slots.len(), 107);
    assert_eq!(count(&|s| facts.rules.customizable(s.role)), 101);
    assert_eq!(count(&|s| s.role == Role::Windup), 5);
    assert_eq!(count(&|s| s.role == Role::Recast), 8);
    assert_eq!(count(&|s| s.role == Role::Release), 68);
    assert_eq!(count(&|s| s.group == Group::Basic), 22);
}

/// Every default OMOBA plays passes the rules a custom clip in its slot must pass, measured
/// on the library's own keys: the rule numbers are tuned against the shipped motions.
#[test]
fn every_default_passes_the_rules_of_its_slot() {
    let facts = facts();
    let leg = f64::from(facts.motion.source_hips_to_feet_distance);
    let param = |rule: &str, name: &str| {
        facts.rules.rule(rule)["params"][name]
            .as_f64()
            .unwrap_or_else(|| panic!("{rule}.{name}"))
    };
    let max_keys = param("clip.keys", "maxKeysPerChannel") as usize;
    let max_rate = param("clip.keys", "maxSampleRateHz");
    let limit = f64::from(facts.timing.contact_limit_secs);
    let tolerance = param("clip.markers", "toleranceSecs");
    let mut failures = Vec::new();
    for (id, slot) in &facts.slots {
        let clip = &facts.motion.clips[&slot.default.motion];
        let d = derive(&facts, slot);
        let window = facts.rules.policy(slot.role).window;
        let played = target_secs(slot, &d);
        if !(window.min_secs..=window.max_secs).contains(&played) {
            failures.push(format!("{id}: plays {played:.3} s outside {window:?}"));
        }
        let keys = clip.times.len();
        let rate = (keys - 1) as f64 / f64::from(clip.duration);
        if keys > max_keys || rate > max_rate + 1e-6 {
            failures.push(format!("{id}: {keys} keys at {rate:.1} Hz"));
        }
        let hold = slot.kind == Kind::Hold && slot.group == Group::Base;
        let horizontal = param(
            "clip.in-place",
            if hold {
                "maxHorizontalHold"
            } else {
                "maxHorizontal"
            },
        );
        for hips in &clip.hips_world_deltas {
            let [x, y, z] = hips.map(f64::from);
            if x.hypot(z) > horizontal * leg
                || y > param("clip.in-place", "maxUp") * leg
                || -y > param("clip.in-place", "maxDown") * leg
            {
                failures.push(format!("{id}: hips leave place ({x:.3}, {y:.3}, {z:.3})"));
                break;
            }
        }
        if slot.kind == Kind::Loop {
            let last = keys - 1;
            for (bone, samples) in &clip.world_rotation_deltas {
                let first = bevy::math::Quat::from_array(samples[0]);
                let end = bevy::math::Quat::from_array(samples[last]);
                let degrees = f64::from(first.angle_between(end).to_degrees());
                if degrees > param("clip.loop-closure", "maxBoneDegrees") {
                    failures.push(format!("{id}: {bone} seam {degrees:.2} deg"));
                }
            }
            let seam = bevy::math::Vec3::from_array(clip.hips_world_deltas[0])
                .distance(bevy::math::Vec3::from_array(clip.hips_world_deltas[last]));
            if f64::from(seam) > param("clip.loop-closure", "maxHipsOffset") * leg {
                failures.push(format!("{id}: hips seam {seam:.4} m"));
            }
        }
        if slot.kind == Kind::Action {
            let Some(contact) = d.contact else {
                failures.push(format!("{id}: no contact time"));
                continue;
            };
            if !(0.0..=d.duration).contains(&contact) {
                failures.push(format!("{id}: contact outside the clip"));
            }
            let late = d.effective_contact.unwrap();
            if slot.edge_released && f64::from(late) > limit + tolerance {
                failures.push(format!("{id}: contact {late:.3} s after the cast"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Numbers the rule table repeats must equal the engine's and the passport's.
#[test]
fn rule_table_agrees_with_the_engine_and_the_passport() {
    let facts = facts();
    let params = |id: &str| facts.rules.rule(id)["params"].clone();
    // The library itself never needs more keys per channel than a custom clip may have.
    let max_keys = params("clip.keys")["maxKeysPerChannel"].as_u64().unwrap();
    assert!(
        facts
            .motion
            .clips
            .values()
            .all(|clip| clip.times.len() as u64 <= max_keys)
    );
    // VRM 1.0 thumbs map onto the library's (VRM 0.x) names, as `retarget::source_semantic`.
    let map = params("clip.channels")["vrm1BoneMap"].clone();
    for (vrm1, library) in map.as_object().unwrap() {
        assert!(vrm1.contains("Thumb"), "{vrm1}");
        assert!(
            facts.motion.bones.iter().any(|bone| bone == library),
            "{library}"
        );
    }
    let ids: Vec<&str> = facts
        .rules
        .rules
        .iter()
        .map(|rule| rule["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "pack.manifest",
            "pack.contract",
            "pack.target",
            "pack.bytes",
            "pack.slot-known",
            "pack.ref",
            "pack.unreferenced",
            "rig.humanoid",
            "clip.channels",
            "clip.sampler",
            "clip.keys",
            "clip.duration",
            "clip.in-place",
            "clip.loop-closure",
            "clip.markers",
            "clip.min-interval",
        ],
        "a new rule id needs both implementations (Studio, passport) before it ships"
    );
    for role in [
        Role::Idle,
        Role::Run,
        Role::Walk,
        Role::Death,
        Role::Basic,
        Role::Release,
        Role::Recast,
        Role::Windup,
    ] {
        let window = facts.rules.policy(role).window;
        assert!(
            0.0 < window.min_secs && window.min_secs < window.max_secs,
            "{role:?}"
        );
    }
    assert_eq!(facts.rules.roles.len(), 8);
}

#[test]
fn stand_ins_are_the_ones_the_animation_readme_names() {
    let readme = std::fs::read_to_string(repo("assets-src/animations/README.md")).unwrap();
    let line = readme
        .split("\n\n")
        .find(|paragraph| paragraph.contains("are stand-ins"))
        .expect("the README names its stand-ins");
    for (motion, _) in STAND_INS {
        assert!(line.contains(&format!("`{motion}`")), "{motion}");
    }
    assert_eq!(line.matches('`').count(), 2 * STAND_INS.len());
}

#[test]
fn wildspark_slots_carry_its_guns() {
    let body = render_contract(&facts());
    let ext = |id: &str| body["slots"][id]["ext"][PROFILE].clone();
    assert_eq!(
        ext("basic.wildspark.0")["handheld"]["item"],
        "wild-repeater"
    );
    assert_eq!(
        ext("basic.wildspark.rockets.0")["handheld"]["item"],
        "wild-launcher"
    );
    assert_eq!(
        ext("skill.wild_rocket.release")["handheld"]["item"],
        "wild-launcher"
    );
    assert_eq!(
        ext("skill.wild_zap.release")["handheld"]["muzzleWindowSecs"],
        json!([0.08, 0.25])
    );
    assert_eq!(
        ext("skill.wild_traps.release")["handheld"]["muzzleWindowSecs"],
        Value::Null
    );
    // Riftshot holds the plain repeater model, which has no barrel flash.
    assert_eq!(
        ext("basic.riftshot.0")["handheld"]["muzzleWindowSecs"],
        Value::Null
    );
    assert_eq!(ext("basic.wildspark.rockets.0")["weaponMode"], "rockets");
    assert_eq!(
        body["slots"]["basic.warden.0"]["alternatesWith"],
        json!(["basic.warden.1"])
    );
    let contact = &body["slots"]["basic.wildspark.0"]["animation"]["markers"]["contact"];
    assert_eq!(
        *contact,
        json!({"required": true, "maxSecs": 0.15, "targetSecs": 0.0455})
    );
    assert_eq!(
        body["slots"]["skill.dawn_ray.release"]["animation"]["leadInSecs"],
        0.8
    );
}

/// The conformance packages isolate one defect each, so their rigs must pass the runtime
/// humanoid check unchanged and load as glTF; the leg length is the unit their in-place
/// cases are built in.
#[test]
fn fixture_packages_are_valid_humanoid_gltf() {
    let directory = repo("shared/assets/catalog/game-contract-fixtures/v1");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    let cases = manifest["packageCases"].as_array().unwrap();
    assert_eq!(cases.len(), 45);
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(case["file"].as_str().unwrap())).unwrap();
        let rig = omoba_passport::humanoid::validate_runtime_humanoid(&bytes)
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        bevy::gltf::gltf::Gltf::from_slice(&bytes).unwrap_or_else(|error| panic!("{id}: {error}"));
        let world = |bone: &str| {
            let mut node = Some(rig.bones[bone]);
            let mut position = bevy::math::Vec3::ZERO;
            while let Some(index) = node {
                position += bevy::math::Vec3::from_array(rig.nodes[index].translation);
                node = rig.nodes[index].parent;
            }
            position
        };
        let feet = (world("leftFoot") + world("rightFoot")) * 0.5;
        assert!(
            (world("hips").y - feet.y - 0.9).abs() < 1e-5,
            "{id}: leg length"
        );
        let left = world("leftHand").x;
        let expected = if rig.version == VrmVersion::Vrm1 {
            1.0
        } else {
            -1.0
        };
        assert_eq!(left.signum(), expected, "{id}: faces as its VRM version");
    }
}
