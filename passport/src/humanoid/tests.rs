use super::*;
use serde_json::json;

fn fixture(version: VrmVersion) -> Value {
    let mut nodes = Vec::new();
    let mut bones = BTreeMap::new();
    for name in required_bones(version) {
        bones.insert(name.to_owned(), nodes.len());
        nodes.push(json!({"name": "deliberately duplicated", "translation": [0, 0.1, 0]}));
    }
    for (name, index) in &bones {
        let mut parent = semantic_parent(name, version);
        while parent
            .as_ref()
            .is_some_and(|parent| !bones.contains_key(parent))
        {
            parent = parent
                .as_deref()
                .and_then(|name| semantic_parent(name, version));
        }
        if let Some(parent) = parent {
            let parent = &mut nodes[bones[&parent]];
            parent
                .as_object_mut()
                .unwrap()
                .entry("children")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .unwrap()
                .push(json!(index));
        }
    }
    let joints: Vec<_> = (0..nodes.len()).collect();
    let mesh = nodes.len();
    nodes.push(json!({"mesh": 0, "skin": 0}));
    let extension = match version {
        VrmVersion::Vrm0 => {
            json!({"VRM": {"specVersion":"0.0", "humanoid": {"humanBones": bones.iter().map(|(bone, node)| json!({"bone":bone,"node":node})).collect::<Vec<_>>()}}})
        }
        VrmVersion::Vrm1 => {
            json!({"VRMC_vrm": {"specVersion":"1.0", "humanoid": {"humanBones": bones.iter().map(|(bone, node)| (bone.clone(), json!({"node":node}))).collect::<serde_json::Map<_,_>>()}}})
        }
    };
    json!({"asset":{"version":"2.0"}, "nodes":nodes, "scenes":[{"nodes":[bones["hips"], mesh]}], "skins":[{"joints":joints}], "meshes":[{"primitives":[]}], "buffers":[{"byteLength":4}], "extensions":extension})
}

fn bytes(document: &Value) -> Vec<u8> {
    let mut json = serde_json::to_vec(document).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let length = (12 + 8 + json.len() + 8 + 4) as u32;
    let mut result = Vec::new();
    result.extend(b"glTF");
    result.extend(2_u32.to_le_bytes());
    result.extend(length.to_le_bytes());
    result.extend((json.len() as u32).to_le_bytes());
    result.extend(b"JSON");
    result.extend(json);
    result.extend(4_u32.to_le_bytes());
    result.extend(b"BIN\0");
    result.extend([0; 4]);
    result
}

fn error_contains(document: &Value, expected: &str) {
    let error = HumanoidRig::from_document(document).unwrap_err();
    assert!(
        error.contains(expected),
        "{error:?} did not contain {expected:?}"
    );
}

#[test]
fn clipless_versions_ignore_names_and_keep_nonidentity_rest_transforms() {
    for version in [VrmVersion::Vrm0, VrmVersion::Vrm1] {
        let mut document = fixture(version);
        document["nodes"][0]["rotation"] = json!([
            0.0,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2
        ]);
        document["nodes"][0]["scale"] = json!([2.0, 2.0, 2.0]);
        let rig = validate_runtime_humanoid(&bytes(&document)).unwrap();
        assert_eq!(rig.version, version);
        assert_eq!(rig.nodes[0].scale, [2.0; 3]);
        assert!((rig.nodes[0].rotation[2] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!(rig.nodes[0].rotation.iter().all(|v| v.is_finite()));
        assert_eq!(
            rig.bones.len(),
            if version == VrmVersion::Vrm0 { 17 } else { 15 }
        );
        assert_eq!(rig.topological_order.len(), rig.nodes.len());
        for (node, descriptor) in rig.nodes.iter().enumerate() {
            if let Some(parent) = descriptor.parent {
                assert!(
                    rig.topological_order.iter().position(|i| *i == parent)
                        < rig.topological_order.iter().position(|i| *i == node)
                );
            }
        }
        // This local skeletal capability must not silently change external admission.
        assert!(
            crate::validate_humanoid_profile(&bytes(&document))
                .unwrap_err()
                .contains("embedded")
        );
    }
}

#[test]
fn all_shipped_vrm_rigs_are_supported_without_modifying_or_using_clips() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../client/assets/avatars");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let avatars = manifest["avatars"].as_array().unwrap();
    assert_eq!(avatars.len(), 15);
    for avatar in avatars {
        let slug = avatar["slug"].as_str().unwrap();
        let data = std::fs::read(root.join(format!("{slug}.glb"))).unwrap();
        let rig = HumanoidRig::from_glb(&data).unwrap_or_else(|error| panic!("{slug}: {error}"));
        assert_eq!(rig.version, VrmVersion::Vrm0);
        assert!(rig.bones.contains_key("leftUpperLeg"));
        let length = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
        let mut document: Value = serde_json::from_slice(&data[20..20 + length]).unwrap();
        document.as_object_mut().unwrap().remove("animations");
        for node in document["nodes"].as_array_mut().unwrap() {
            node["name"] = json!("same");
        }
        assert_eq!(HumanoidRig::from_document(&document).unwrap(), rig);
    }
}

#[test]
fn vrm1_optional_chest_neck_and_thumb_names_follow_the_spec() {
    let mut document = fixture(VrmVersion::Vrm1);
    let first = HumanoidRig::from_document(&document).unwrap();
    assert!(!first.bones.contains_key("chest"));
    assert!(!first.bones.contains_key("neck"));
    let hand = first.bones["leftHand"];
    let start = document["nodes"].as_array().unwrap().len();
    document["nodes"][hand]["children"] = json!([start]);
    for (offset, name) in [
        "leftThumbMetacarpal",
        "leftThumbProximal",
        "leftThumbDistal",
    ]
    .iter()
    .enumerate()
    {
        let index = start + offset;
        document["nodes"]
            .as_array_mut()
            .unwrap()
            .push(if offset < 2 {
                json!({"children":[index+1]})
            } else {
                json!({})
            });
        document["skins"][0]["joints"]
            .as_array_mut()
            .unwrap()
            .push(json!(index));
        document["extensions"]["VRMC_vrm"]["humanoid"]["humanBones"][*name] = json!({"node":index});
    }
    let rig = HumanoidRig::from_document(&document).unwrap();
    assert_eq!(rig.bones["leftThumbMetacarpal"], start);
    assert_eq!(rig.bones["leftThumbProximal"], start + 1);
    assert!(!rig.bones.contains_key("leftThumbIntermediate"));
}

#[test]
fn intermediary_nodes_are_permitted_without_name_guessing() {
    let mut document = fixture(VrmVersion::Vrm1);
    let rig = HumanoidRig::from_document(&document).unwrap();
    let upper = rig.bones["leftUpperLeg"];
    let lower = rig.bones["leftLowerLeg"];
    let middle = document["nodes"].as_array().unwrap().len();
    document["nodes"][upper]["children"] = json!([middle]);
    document["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"unmapped helper", "rotation":[0.2,0,0,0.9797958971132712],"children":[lower]}));
    let rig = HumanoidRig::from_document(&document).unwrap();
    assert_eq!(rig.nodes[lower].parent, Some(middle));
}

#[test]
fn malformed_maps_and_unsupported_rigs_fail_with_context() {
    let base = fixture(VrmVersion::Vrm1);
    let rig = HumanoidRig::from_document(&base).unwrap();
    let cases: Vec<(&str, Value)> = vec![
        ("missing required bone", {
            let mut d = base.clone();
            d["extensions"]["VRMC_vrm"]["humanoid"]["humanBones"]
                .as_object_mut()
                .unwrap()
                .remove("head");
            d
        }),
        ("out of range", {
            let mut d = base.clone();
            d["extensions"]["VRMC_vrm"]["humanoid"]["humanBones"]["head"]["node"] = json!(99999);
            d
        }),
        ("duplicated", {
            let mut d = base.clone();
            d["extensions"]["VRMC_vrm"]["humanoid"]["humanBones"]["head"]["node"] =
                json!(rig.bones["hips"]);
            d
        }),
        ("multiple parents", {
            let mut d = base.clone();
            d["nodes"][rig.bones["leftHand"]]["children"] = json!([rig.bones["rightHand"]]);
            d
        }),
        ("cycle", {
            let mut d = base.clone();
            let i = d["nodes"].as_array().unwrap().len();
            d["nodes"]
                .as_array_mut()
                .unwrap()
                .extend([json!({"children":[i+1]}), json!({"children":[i]})]);
            d
        }),
        ("positive uniform", {
            let mut d = base.clone();
            d["nodes"][0]["scale"] = json!([1, 2, 1]);
            d
        }),
        ("positive uniform", {
            let mut d = base.clone();
            d["nodes"][0]["scale"] = json!([-1, -1, -1]);
            d
        }),
        ("positive uniform", {
            let mut d = base.clone();
            d["nodes"][0]["scale"] = json!([0, 0, 0]);
            d
        }),
        ("zero rotation", {
            let mut d = base.clone();
            d["nodes"][0]["rotation"] = json!([0, 0, 0, 0]);
            d
        }),
        ("finite", {
            let mut d = base.clone();
            d["nodes"][0]["rotation"] = json!([1e100, 0, 0, 1]);
            d
        }),
        ("matrix", {
            let mut d = base.clone();
            d["nodes"][0]["matrix"] = json!(vec![1; 16]);
            d
        }),
        ("embedded", {
            let mut d = base.clone();
            d["buffers"][0]["uri"] = json!("https://example.invalid/model.bin");
            d
        }),
        ("embedded", {
            let mut d = base.clone();
            d["images"] = json!([{"uri":"data:image/png;base64,AAAA"}]);
            d
        }),
        ("not a skin joint", {
            let mut d = base.clone();
            d["skins"][0]["joints"]
                .as_array_mut()
                .unwrap()
                .retain(|i| i.as_u64() != Some(rig.bones["head"] as u64));
            d
        }),
        ("not instantiated", {
            let mut d = base.clone();
            d["scenes"][0]["nodes"].as_array_mut().unwrap().remove(0);
            d
        }),
        ("incompatible parent", {
            let mut d = base.clone();
            let lower = rig.bones["leftLowerLeg"];
            d["nodes"][rig.bones["leftUpperLeg"]]["children"] = json!([]);
            d["nodes"][rig.bones["leftHand"]]["children"] = json!([lower]);
            d
        }),
        ("conflicting", {
            let mut d = base.clone();
            d["extensions"]["VRM"] = json!({});
            d
        }),
        ("specVersion", {
            let mut d = base.clone();
            d["extensions"]["VRMC_vrm"]["specVersion"] = json!("2.0");
            d
        }),
    ];
    for (expected, document) in cases {
        error_contains(&document, expected);
    }
}

#[test]
fn malformed_container_and_limits_are_rejected_without_panics() {
    let document = fixture(VrmVersion::Vrm1);
    let good = bytes(&document);
    for length in 0..good.len() {
        assert!(HumanoidRig::from_glb(&good[..length]).is_err());
    }
    for offset in [0, 4, 8, 12, 16] {
        let mut corrupt = good.clone();
        corrupt[offset..offset + 4].fill(255);
        assert!(HumanoidRig::from_glb(&corrupt).is_err());
    }
    let mut excessive = document.clone();
    excessive["nodes"] = json!(vec![json!({}); MAX_HUMANOID_NODES + 1]);
    error_contains(&excessive, "4096");
    let mut wrong_buffer = document.clone();
    wrong_buffer["buffers"][0]["byteLength"] = json!(100);
    assert!(
        HumanoidRig::from_glb(&bytes(&wrong_buffer))
            .unwrap_err()
            .contains("buffer length")
    );
    let mut oversized_json = document;
    oversized_json["extras"] = json!("x".repeat(MAX_HUMANOID_JSON_BYTES));
    assert!(
        HumanoidRig::from_glb(&bytes(&oversized_json))
            .unwrap_err()
            .contains("4 MiB")
    );
}

#[test]
fn required_extensions_match_the_supported_renderer_subset() {
    for version in [VrmVersion::Vrm0, VrmVersion::Vrm1] {
        let mut document = fixture(version);
        document["extensionsUsed"] = json!([match version {
            VrmVersion::Vrm0 => "VRM",
            VrmVersion::Vrm1 => "VRMC_vrm",
        }]);
        assert!(HumanoidRig::from_document(&document).is_ok());
        for extension in [
            "VRM",
            "VRMC_vrm",
            "VRMC_materials_mtoon",
            "VRMC_springBone",
            "VRMC_node_constraint",
            "VENDOR_unknown",
            "KHR_draco_mesh_compression",
        ] {
            document["extensionsRequired"] = json!([extension]);
            let error = HumanoidRig::from_glb(&bytes(&document)).unwrap_err();
            assert!(error.contains(extension), "{error}");
            assert!(
                error.contains("unsupported by the current renderer"),
                "{error}"
            );
        }
        document["extensionsRequired"] = json!(["KHR_materials_unlit", "KHR_texture_transform"]);
        document["extensionsUsed"] = document["extensionsRequired"].clone();
        assert!(HumanoidRig::from_glb(&bytes(&document)).is_ok());
        document["extensionsRequired"] = json!([null]);
        error_contains(&document, "extension names");
        document["extensionsRequired"] = json!("VRMC_vrm");
        error_contains(&document, "array");
    }
}

#[test]
fn nonunit_source_quaternions_are_rejected_instead_of_repaired_only_in_metadata() {
    for version in [VrmVersion::Vrm0, VrmVersion::Vrm1] {
        for rotation in [
            [0.0, 0.0, 2.0, 2.0],
            [0.0, 0.0, 1e20, 1e20],
            [0.0, 0.0, 0.0, 0.5],
        ] {
            let mut document = fixture(version);
            document["nodes"][0]["rotation"] = json!(rotation);
            let original = document.clone();
            error_contains(&document, "unit rotation quaternion");
            assert!(
                HumanoidRig::from_glb(&bytes(&document))
                    .unwrap_err()
                    .contains("unit rotation quaternion")
            );
            assert_eq!(document, original);
        }
        let mut rounded = fixture(version);
        rounded["nodes"][0]["rotation"] = json!([0.0, 0.0, 0.0, 1.00001]);
        let rig = HumanoidRig::from_document(&rounded).unwrap();
        assert_eq!(rig.nodes[0].rotation, [0.0, 0.0, 0.0, 1.0]);
    }
}
