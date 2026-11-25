// The Blender exports for this prototype are GLB 2.0 triangle meshes with solid materials.
export const identity = new Float32Array([
  1, 0, 0, 0,
  0, 1, 0, 0,
  0, 0, 1, 0,
  0, 0, 0, 1,
]);

export function multiply(left, right) {
  const result = new Float32Array(16);
  for (let column = 0; column < 4; column++) {
    for (let row = 0; row < 4; row++) {
      for (let index = 0; index < 4; index++) {
        result[column * 4 + row] += left[index * 4 + row] * right[column * 4 + index];
      }
    }
  }
  return result;
}

function localMatrix(node) {
  if (node.matrix) return new Float32Array(node.matrix);
  const [x, y, z, w] = node.rotation ?? [0, 0, 0, 1];
  const [sx, sy, sz] = node.scale ?? [1, 1, 1];
  const [tx, ty, tz] = node.translation ?? [0, 0, 0];
  return new Float32Array([
    (1 - 2 * (y * y + z * z)) * sx, 2 * (x * y + z * w) * sx, 2 * (x * z - y * w) * sx, 0,
    2 * (x * y - z * w) * sy, (1 - 2 * (x * x + z * z)) * sy, 2 * (y * z + x * w) * sy, 0,
    2 * (x * z + y * w) * sz, 2 * (y * z - x * w) * sz, (1 - 2 * (x * x + y * y)) * sz, 0,
    tx, ty, tz, 1,
  ]);
}

const componentFormat = {
  5121: [1, "getUint8"],
  5123: [2, "getUint16"],
  5125: [4, "getUint32"],
  5126: [4, "getFloat32"],
};

function unpack(document, binary, accessorIndex, width, resultType) {
  const accessor = document.accessors[accessorIndex];
  const bufferView = document.bufferViews[accessor.bufferView];
  const [componentBytes, method] = componentFormat[accessor.componentType];
  const stride = bufferView.byteStride ?? width * componentBytes;
  const offset = (bufferView.byteOffset ?? 0) + (accessor.byteOffset ?? 0);
  const values = new resultType(accessor.count * width);

  for (let item = 0; item < accessor.count; item++) {
    for (let component = 0; component < width; component++) {
      values[item * width + component] =
        binary[method](offset + item * stride + component * componentBytes, true);
    }
  }
  return values;
}

export async function loadGlb(url, embeddedId) {
  const embedded = document.getElementById(embeddedId);
  if (embedded) {
    const bytes = Uint8Array.from(atob(embedded.textContent.trim()), character => character.charCodeAt(0));
    return readGlb(bytes.buffer, url);
  }
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url} returned HTTP ${response.status}`);
  return readGlb(await response.arrayBuffer(), url);
}

function readGlb(file, url) {
  const header = new DataView(file);
  if (header.getUint32(0, true) !== 0x46546c67 || header.getUint32(4, true) !== 2) {
    throw new Error(`Expected GLB 2.0 geometry in ${url}`);
  }
  const jsonLength = header.getUint32(12, true);
  const document = JSON.parse(new TextDecoder().decode(new Uint8Array(file, 20, jsonLength)));
  const binaryStart = 20 + jsonLength;
  const binary = new DataView(file, binaryStart + 8, header.getUint32(binaryStart, true));
  const primitives = [];

  function visit(nodeIndex, parent) {
    const node = document.nodes[nodeIndex];
    const matrix = multiply(parent, localMatrix(node));
    if (node.mesh !== undefined) {
      for (const primitive of document.meshes[node.mesh].primitives) {
        const { attributes, indices, mode = 4 } = primitive;
        if (mode !== 4 || attributes.NORMAL === undefined) {
          throw new Error(`Expected triangles with normals in ${url}`);
        }
        const material = document.materials?.[primitive.material];
        primitives.push({
          matrix,
          nodeName: node.name ?? "",
          material: material?.name,
          color: material?.pbrMetallicRoughness?.baseColorFactor ?? [1, 1, 1, 1],
          positions: unpack(document, binary, attributes.POSITION, 3, Float32Array),
          normals: unpack(document, binary, attributes.NORMAL, 3, Float32Array),
          indices: indices === undefined ? null : unpack(document, binary, indices, 1, Uint32Array),
        });
      }
    }
    for (const child of node.children ?? []) visit(child, matrix);
  }

  for (const node of document.scenes[document.scene ?? 0].nodes) visit(node, identity);
  return primitives;
}
