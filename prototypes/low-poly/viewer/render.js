import { identity, multiply } from "./glb.js";

function linearChannel(channel) {
  return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
}

export function color(rgb, alpha = 1) {
  return [16, 8, 0].map(shift => linearChannel(((rgb >> shift) & 255) / 255)).concat(alpha);
}

const vertexSource = `#version 300 es
layout (location = 0) in vec3 position;
layout (location = 1) in vec3 normal;
uniform mat4 modelMatrix;
uniform mat4 viewProjection;
out vec3 worldNormal;

void main() {
  worldNormal = mat3(transpose(inverse(modelMatrix))) * normal;
  gl_Position = viewProjection * modelMatrix * vec4(position, 1.0);
}`;

const fragmentSource = `#version 300 es
precision highp float;
in vec3 worldNormal;
uniform vec4 baseColor;
uniform float lighting;
out vec4 pixel;

void main() {
  vec3 sun = normalize(vec3(-0.5, 0.9, 0.45));
  float diffuse = max(dot(normalize(worldNormal), sun), 0.0);
  vec3 linear = baseColor.rgb * mix(1.0, 0.8 + 0.23 * diffuse, lighting);
  vec3 srgb = mix(12.92 * linear, 1.055 * pow(linear, vec3(1.0 / 2.4)) - 0.055,
                  step(vec3(0.0031308), linear));
  pixel = vec4(srgb, baseColor.a);
}`;

export function makeProgram(gl) {
  const program = gl.createProgram();
  for (const [type, source] of [[gl.VERTEX_SHADER, vertexSource], [gl.FRAGMENT_SHADER, fragmentSource]]) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader));
    gl.attachShader(program, shader);
    gl.deleteShader(shader);
  }
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program));
  return program;
}

export function upload(gl, primitive) {
  const vao = gl.createVertexArray();
  gl.bindVertexArray(vao);
  for (const [attribute, values] of [[0, primitive.positions], [1, primitive.normals]]) {
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, values, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(attribute);
    gl.vertexAttribPointer(attribute, 3, gl.FLOAT, false, 0, 0);
  }
  if (primitive.indices) {
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, primitive.indices, gl.STATIC_DRAW);
  }
  gl.bindVertexArray(null);
  return {
    vao,
    matrix: primitive.matrix,
    material: primitive.material,
    color: primitive.color,
    indexed: Boolean(primitive.indices),
    count: primitive.indices?.length ?? primitive.positions.length / 3,
  };
}

export function circle(gl, innerRadius) {
  const segments = 28;
  const positions = [];
  const normals = [];
  const indices = [];
  for (let step = 0; step <= segments; step++) {
    const angle = step / segments * Math.PI * 2;
    const x = Math.cos(angle);
    const z = Math.sin(angle);
    positions.push(x, 0, z, x * innerRadius, 0, z * innerRadius);
    normals.push(0, 1, 0, 0, 1, 0);
    if (step > 0) {
      const index = step * 2;
      indices.push(index - 2, index - 1, index, index, index - 1, index + 1);
    }
  }
  return upload(gl, {
    matrix: identity,
    positions: new Float32Array(positions),
    normals: new Float32Array(normals),
    indices: new Uint32Array(indices),
  });
}

function normalize(vector) {
  const length = Math.hypot(...vector);
  return vector.map(component => component / length);
}

function cross(left, right) {
  return [
    left[1] * right[2] - left[2] * right[1],
    left[2] * right[0] - left[0] * right[2],
    left[0] * right[1] - left[1] * right[0],
  ];
}

function dot(left, right) {
  return left[0] * right[0] + left[1] * right[1] + left[2] * right[2];
}

export function makeCamera(aspect, cameraEye, cameraTarget, minimumWidth = 21.8, minimumHeight = 11.1) {
  const back = normalize(cameraEye.map((component, axis) => component - cameraTarget[axis]));
  const right = normalize(cross([0, 1, 0], back));
  const up = cross(back, right);
  const view = new Float32Array([
    right[0], up[0], back[0], 0,
    right[1], up[1], back[1], 0,
    right[2], up[2], back[2], 0,
    -dot(right, cameraEye), -dot(up, cameraEye), -dot(back, cameraEye), 1,
  ]);
  const halfHeight = Math.max(minimumHeight, minimumWidth / aspect);
  const halfWidth = halfHeight * aspect;
  const near = 0.1;
  const far = 100;
  const projection = new Float32Array([
    1 / halfWidth, 0, 0, 0,
    0, 1 / halfHeight, 0, 0,
    0, 0, -2 / (far - near), 0,
    0, 0, -(far + near) / (far - near), 1,
  ]);
  return { right, up, back, halfWidth, halfHeight, viewProjection: multiply(projection, view) };
}

export function transform(x, y, z, yaw, scaleX = 1, scaleZ = 1, scaleY = 1) {
  const cosine = Math.cos(yaw);
  const sine = Math.sin(yaw);
  return new Float32Array([
    cosine * scaleX, 0, -sine * scaleX, 0,
    0, scaleY, 0, 0,
    sine * scaleZ, 0, cosine * scaleZ, 0,
    x, y, z, 1,
  ]);
}
