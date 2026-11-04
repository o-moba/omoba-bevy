//! Deliberately small map-prop v1: embedded, unskinned material/vertex-color GLB.
//! Read real vertex/index/keyframe bytes before the Bevy loader sees the file.
use serde_json::Value;

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelFit {
    pub scale: f32,
    pub offset: [f32; 3],
}
fn error() -> String {
    "Map prop exceeds the safe GLB v1 profile".into()
}
fn array<'a>(d: &'a Value, key: &str, max: usize) -> Result<&'a Vec<Value>, String> {
    d[key]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= max)
        .ok_or_else(error)
}
fn index(v: &Value, n: usize) -> Result<usize, String> {
    v.as_u64()
        .and_then(|i| usize::try_from(i).ok())
        .filter(|i| *i < n)
        .ok_or_else(error)
}
fn number(v: &Value) -> Result<f32, String> {
    v.as_f64()
        .map(|v| v as f32)
        .filter(|v| v.is_finite() && v.abs() <= 10000.)
        .ok_or_else(error)
}
fn vector<const N: usize>(v: Option<&Value>, default: [f32; N]) -> Result<[f32; N], String> {
    let Some(v) = v else { return Ok(default) };
    let a = v.as_array().filter(|a| a.len() == N).ok_or_else(error)?;
    let mut result = [0.; N];
    for (i, v) in a.iter().enumerate() {
        result[i] = number(v)?;
    }
    Ok(result)
}
struct Data<'a> {
    d: &'a Value,
    bin: &'a [u8],
}
impl Data<'_> {
    fn values(&self, accessor: &Value, expected: &str) -> Result<Vec<Vec<f32>>, String> {
        let a = &array(self.d, "accessors", 1024)?
            [index(accessor, self.d["accessors"].as_array().map_or(0, Vec::len))?];
        if a["type"] != expected
            || a.get("sparse").is_some()
            || a.get("normalized").is_some_and(|v| v != false)
        {
            return Err(error());
        }
        let width = match expected {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => return Err(error()),
        };
        let count = a["count"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= 150000)
            .ok_or_else(error)? as usize;
        let component = a["componentType"].as_u64().ok_or_else(error)?;
        let unit = match component {
            5121 => 1,
            5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(error()),
        };
        let views = array(self.d, "bufferViews", 1024)?;
        let view = &views[index(&a["bufferView"], views.len())?];
        if view["buffer"] != 0 {
            return Err(error());
        }
        let offset = |v: Option<&Value>| -> Result<usize, String> {
            v.map_or(Ok(0), |v| {
                v.as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or_else(error)
            })
        };
        let start = offset(view.get("byteOffset"))?;
        let len = view["byteLength"]
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(error)?;
        let end = start
            .checked_add(len)
            .filter(|v| *v <= self.bin.len())
            .ok_or_else(error)?;
        let within = offset(a.get("byteOffset"))?;
        let stride = view.get("byteStride").map_or(Ok(width * unit), |v| {
            v.as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(error)
        })?;
        if stride < width * unit || stride > 252 || within > len {
            return Err(error());
        }
        let first = start.checked_add(within).ok_or_else(error)?;
        if first
            .checked_add((count - 1).checked_mul(stride).ok_or_else(error)?)
            .and_then(|n| n.checked_add(width * unit))
            .is_none_or(|n| n > end)
        {
            return Err(error());
        }
        let mut result = Vec::with_capacity(count);
        for i in 0..count {
            let mut values = Vec::with_capacity(width);
            for c in 0..width {
                let at = first + i * stride + c * unit;
                let n = match component {
                    5121 => self.bin[at] as f32,
                    5123 => u16::from_le_bytes(self.bin[at..at + 2].try_into().unwrap()) as f32,
                    5125 => u32::from_le_bytes(self.bin[at..at + 4].try_into().unwrap()) as f32,
                    _ => f32::from_le_bytes(self.bin[at..at + 4].try_into().unwrap()),
                };
                if !n.is_finite() {
                    return Err(error());
                }
                values.push(n);
            }
            result.push(values);
        }
        Ok(result)
    }
}
fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let u = [q[0], q[1], q[2]];
    let a = cross(u, v);
    let b = cross(u, a);
    std::array::from_fn(|i| v[i] + 2. * (q[3] * a[i] + b[i]))
}
/// Returns a fit inside a fixed four-metre envelope, with no authored collider.
pub fn validate(bytes: &[u8]) -> Result<ModelFit, String> {
    let rules = ekza_bevy_sdk::GlbValidationRules {
        max_bytes: Some(MAX_BYTES),
        allow_trailing_bytes: false,
    };
    if bytes.len() < 28 || !ekza_bevy_sdk::validate_glb_bytes(bytes, &rules).is_valid() {
        return Err(error());
    }
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let bin_header = 20_usize
        .checked_add(json_len)
        .filter(|n| n + 8 <= bytes.len())
        .ok_or_else(error)?;
    if bytes.get(16..20) != Some(b"JSON")
        || bytes.get(bin_header + 4..bin_header + 8) != Some(b"BIN\0")
        || !json_len.is_multiple_of(4)
    {
        return Err(error());
    }
    let bin_len =
        u32::from_le_bytes(bytes[bin_header..bin_header + 4].try_into().unwrap()) as usize;
    if bin_header + 8 + bin_len != bytes.len() {
        return Err(error());
    }
    let d: Value = serde_json::from_slice(&bytes[20..bin_header]).map_err(|_| error())?;
    if d["asset"]["version"] != "2.0" || d.get("scene").is_some_and(|v| v != 0) {
        return Err(error());
    }
    for key in [
        "skins",
        "images",
        "textures",
        "cameras",
        "extensionsRequired",
        "extensionsUsed",
    ] {
        if d.get(key)
            .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
        {
            return Err(error());
        }
    }
    if d.get("extensions").is_some() {
        return Err(error());
    }
    let buffers = array(&d, "buffers", 1)?;
    if buffers[0].get("uri").is_some()
        || buffers[0]["byteLength"]
            .as_u64()
            .is_none_or(|n| n as usize > bin_len || bin_len - n as usize > 3)
    {
        return Err(error());
    }
    let nodes = array(&d, "nodes", 64)?;
    let meshes = array(&d, "meshes", 16)?;
    if nodes.iter().any(|n| !n.is_object()) {
        return Err(error());
    }
    array(&d, "scenes", 1)?;
    crate::weapons::validate_scene_graph(&d)?;
    if d.get("materials")
        .is_some_and(|v| v.as_array().is_none_or(|a| a.len() > 16))
    {
        return Err(error());
    }
    let data = Data {
        d: &d,
        bin: &bytes[bin_header + 8..],
    };
    let mut points = vec![Vec::<[f32; 3]>::new(); meshes.len()];
    let mut triangles = 0;
    let mut vertices = 0;
    let mut mesh_triangles = Vec::new();
    for (mesh, out) in meshes.iter().zip(&mut points) {
        if mesh.get("weights").is_some() {
            return Err(error());
        }
        let before = triangles;
        for p in array(mesh, "primitives", 64)? {
            if p.get("targets").is_some()
                || p.get("extensions").is_some()
                || p.get("mode").is_some_and(|v| v != 4)
            {
                return Err(error());
            }
            let positions = data.values(&p["attributes"]["POSITION"], "VEC3")?;
            let accessor = &d["accessors"][index(
                &p["attributes"]["POSITION"],
                d["accessors"].as_array().unwrap().len(),
            )?];
            if accessor["componentType"] != 5126
                || positions.iter().flatten().any(|v| v.abs() > 1000.)
            {
                return Err(error());
            }
            vertices += positions.len();
            if vertices > 100000 {
                return Err(error());
            }
            let count = if let Some(i) = p.get("indices") {
                if !matches!(d["accessors"][index(i,d["accessors"].as_array().unwrap().len())?]["componentType"].as_u64(),Some(5121|5123|5125)){return Err(error());}
                let indices = data.values(i, "SCALAR")?;
                if indices
                    .iter()
                    .any(|v| v[0] < 0. || v[0] as usize >= positions.len())
                {
                    return Err(error());
                }
                indices.len()
            } else {
                positions.len()
            };
            if count % 3 != 0 {
                return Err(error());
            }
            triangles += count / 3;
            if triangles > 50000 {
                return Err(error());
            }
            out.extend(positions.iter().map(|v| [v[0], v[1], v[2]]));
        }
        mesh_triangles.push(triangles - before);
    }
    let instance_triangles: usize = nodes
        .iter()
        .filter_map(|n| n["mesh"].as_u64())
        .map(|n| mesh_triangles[n as usize])
        .sum();
    if instance_triangles > 50000 {
        return Err(error());
    }
    let mut transforms = Vec::new();
    for n in nodes {
        if n.get("matrix").is_some()
            || n.get("skin").is_some()
            || n.get("camera").is_some()
            || n.get("extensions").is_some()
        {
            return Err(error());
        }
        let t = vector(n.get("translation"), [0.; 3])?;
        let q = vector(n.get("rotation"), [0., 0., 0., 1.])?;
        let s = vector(n.get("scale"), [1.; 3])?;
        if (q.iter().map(|v| v * v).sum::<f32>() - 1.).abs() > 0.01
            || s.iter().any(|v| *v <= 0. || *v > 100.)
        {
            return Err(error());
        }
        transforms.push((t, q, s));
    }
    // Radius calculation deliberately ignores rotations, enclosing every pose.
    fn radius(
        i: usize,
        nodes: &[Value],
        points: &[Vec<[f32; 3]>],
        trs: &[([f32; 3], [f32; 4], [f32; 3])],
    ) -> f32 {
        let mesh = nodes[i]["mesh"]
            .as_u64()
            .map(|n| {
                points[n as usize]
                    .iter()
                    .map(|v| length(*v))
                    .fold(0., f32::max)
            })
            .unwrap_or(0.);
        let child = nodes[i]["children"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|v| radius(v.as_u64().unwrap() as usize, nodes, points, trs))
                    .fold(0., f32::max)
            })
            .unwrap_or(0.);
        let (t, _, s) = trs[i];
        length(t) + s.into_iter().fold(0., f32::max) * mesh.max(child)
    }
    let roots = d["scenes"][0]["nodes"].as_array().unwrap();
    let r = roots
        .iter()
        .map(|v| radius(v.as_u64().unwrap() as usize, nodes, &points, &transforms))
        .fold(0., f32::max);
    if !r.is_finite() || !(0.01..=10000.).contains(&r) {
        return Err(error());
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    fn visit(
        i: usize,
        path: &mut Vec<usize>,
        nodes: &[Value],
        points: &[Vec<[f32; 3]>],
        trs: &[([f32; 3], [f32; 4], [f32; 3])],
        min: &mut [f32; 3],
        max: &mut [f32; 3],
    ) {
        path.push(i);
        if let Some(mesh) = nodes[i]["mesh"].as_u64() {
            for p in &points[mesh as usize] {
                let mut p = *p;
                for j in path.iter().rev() {
                    let (t, q, s) = trs[*j];
                    p = rotate(q, std::array::from_fn(|a| p[a] * s[a]));
                    for a in 0..3 {
                        p[a] += t[a];
                    }
                }
                for a in 0..3 {
                    min[a] = min[a].min(p[a]);
                    max[a] = max[a].max(p[a]);
                }
            }
        }
        if let Some(children) = nodes[i]["children"].as_array() {
            for child in children {
                visit(
                    child.as_u64().unwrap() as usize,
                    path,
                    nodes,
                    points,
                    trs,
                    min,
                    max,
                );
            }
        }
        path.pop();
    }
    for root in roots {
        visit(
            root.as_u64().unwrap() as usize,
            &mut vec![],
            nodes,
            &points,
            &transforms,
            &mut min,
            &mut max,
        );
    }
    if !min.iter().chain(&max).all(|n| n.is_finite()) || max[1] - min[1] < 0.001 {
        return Err(error());
    }
    if let Some(animations) = d.get("animations") {
        let animations = animations
            .as_array()
            .filter(|a| a.len() <= 1)
            .ok_or_else(error)?;
        for anim in animations {
            let samplers = array(anim, "samplers", 64)?;
            for channel in array(anim, "channels", 64)? {
                let node = index(&channel["target"]["node"], nodes.len())?;
                if channel["target"]["path"] != "rotation" || nodes[node].get("matrix").is_some() {
                    return Err(error());
                }
                let sampler = &samplers[index(&channel["sampler"], samplers.len())?];
                if sampler
                    .get("interpolation")
                    .is_some_and(|v| v != "LINEAR" && v != "STEP")
                {
                    return Err(error());
                }
                for key in ["input", "output"] {
                    if d["accessors"]
                        [index(&sampler[key], d["accessors"].as_array().unwrap().len())?]["componentType"]
                        != 5126
                    {
                        return Err(error());
                    }
                }
                let times = data.values(&sampler["input"], "SCALAR")?;
                let values = data.values(&sampler["output"], "VEC4")?;
                if times.len() != values.len()
                    || times.len() > 1024
                    || times.len() < 2
                    || times[0][0] < 0.
                    || times.last().unwrap()[0] > 30.
                    || times.windows(2).any(|v| v[0][0] >= v[1][0])
                    || values
                        .iter()
                        .any(|q| (q.iter().map(|v| v * v).sum::<f32>() - 1.).abs() > 0.01)
                {
                    return Err(error());
                }
            }
        }
    }
    let scale = (2.5 / (max[1] - min[1])).min(shared::sponsorship::MODEL_ENVELOPE_RADIUS / r);
    Ok(ModelFit {
        scale,
        offset: [0., -min[1] * scale, 0.],
    })
}
