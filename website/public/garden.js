// Garden diorama for the hero: Prune (the gardener) points out dead branches, the agent (the
// robot) snips them, and they fall away. Decorative only; all copy is HTML.
//
// Art direction: a refined miniature. Smooth clay forms for characters and the island top,
// small gem-like facets only on foliage and rock, soft image-based light, baked gradients,
// and contact shadows instead of hard edges.
import * as THREE from "three";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";
import { RoundedBoxGeometry } from "three/addons/geometries/RoundedBoxGeometry.js";

const C = {
  grassTop: 0xa9c98a,
  grassEdge: 0x86ad6a,
  grassSide: 0x7a9f5f,
  soil: 0xb9876a,
  soilDeep: 0x93644c,
  rock: 0x7c726d,
  rockLight: 0x9d938b,
  stone: 0xd9d2c4,
  trunk: 0x8a6046,
  leafDark: 0x4f7d4c,
  leafLight: 0x9cc77a,
  shrubDark: 0x5c8a55,
  shrubLight: 0xa6cf86,
  dead: 0xcdbca6,
  deadLeaf: 0xc49a64,
  ribbon: 0xd9643a,
  tag: 0xf6efe2,
  skin: 0xf2c6a2,
  cheek: 0xeb9c8c,
  hat: 0xe9cf96,
  hatBand: 0xc9573a,
  shirt: 0x4d7896,
  apron: 0xe2d4b6,
  pants: 0x39414d,
  shoes: 0x6a4a35,
  clipboard: 0xb98a5c,
  paper: 0xfbf7ee,
  robot: 0xf5f3ef,
  visor: 0x1d2329,
  glow: 0xc9f26b,
  amber: 0xf08a4b,
  steel: 0xdfe5ea,
  flowers: [0xf5d36b, 0xf2a3a0, 0xffffff, 0xc5b3f2],
};

const DEAD_BRANCHES = [
  { label: "orphan.ts", note: "unreachable file" },
  { label: "formatDate()", note: "unused export" },
  { label: "UserDTO", note: "duplicate type" },
];

const UP = new THREE.Vector3(0, 1, 0);

function seeded(seed) {
  let state = seed % 2147483647;
  return () => (state = (state * 16807) % 2147483647) / 2147483647;
}

function clay(color, extra = {}) {
  return new THREE.MeshStandardMaterial({ color, roughness: 0.78, metalness: 0, ...extra });
}

function facets(extra = {}) {
  return new THREE.MeshStandardMaterial({ vertexColors: true, flatShading: true, roughness: 0.85, ...extra });
}

function shadowed(mesh, cast = true, receive = true) {
  mesh.castShadow = cast;
  mesh.receiveShadow = receive;
  return mesh;
}

/** Offset vertices randomly; coincident vertices move together so faces stay closed. */
function jitter(geometry, amount, rand) {
  const position = geometry.attributes.position;
  const offsets = new Map();
  const v = new THREE.Vector3();
  for (let i = 0; i < position.count; i++) {
    v.fromBufferAttribute(position, i);
    const key = `${v.x.toFixed(3)},${v.y.toFixed(3)},${v.z.toFixed(3)}`;
    let offset = offsets.get(key);
    if (!offset) {
      offset = new THREE.Vector3((rand() - 0.5) * amount, (rand() - 0.5) * amount, (rand() - 0.5) * amount);
      offsets.set(key, offset);
    }
    position.setXYZ(i, v.x + offset.x, v.y + offset.y, v.z + offset.z);
  }
  position.needsUpdate = true;
  geometry.computeVertexNormals();
  return geometry;
}

/** Bake a vertical gradient into vertex colors (a cheap stand-in for ambient occlusion). */
function gradient(geometry, bottom, top, curve = 1) {
  geometry.computeBoundingBox();
  const { min, max } = geometry.boundingBox;
  const position = geometry.attributes.position;
  const colors = new Float32Array(position.count * 3);
  const a = new THREE.Color(bottom);
  const b = new THREE.Color(top);
  const c = new THREE.Color();
  for (let i = 0; i < position.count; i++) {
    const t = Math.pow((position.getY(i) - min.y) / Math.max(1e-6, max.y - min.y), curve);
    c.copy(a).lerp(b, t);
    colors.set([c.r, c.g, c.b], i * 3);
  }
  geometry.setAttribute("color", new THREE.BufferAttribute(colors, 3));
  return geometry;
}

/** Lathe a profile, nudging the radius with a low-frequency wobble so the island looks organic. */
function organicLathe(points, segments, wobble) {
  const geometry = new THREE.LatheGeometry(points.map(([x, y]) => new THREE.Vector2(x, y)), segments);
  const position = geometry.attributes.position;
  for (let i = 0; i < position.count; i++) {
    const x = position.getX(i);
    const z = position.getZ(i);
    const k = 1 + wobble(Math.atan2(z, x));
    position.setX(i, x * k);
    position.setZ(i, z * k);
  }
  geometry.computeVertexNormals();
  return geometry;
}

function radialTexture(stops) {
  const size = 128;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const ctx = canvas.getContext("2d");
  const g = ctx.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  for (const [at, color] of stops) g.addColorStop(at, color);
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, size, size);
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}

function contactShadow(texture, radius, opacity = 1) {
  const mesh = new THREE.Mesh(
    new THREE.PlaneGeometry(radius * 2, radius * 2),
    new THREE.MeshBasicMaterial({ map: texture, transparent: true, depthWrite: false, opacity })
  );
  mesh.rotation.x = -Math.PI / 2;
  mesh.position.y = 0.012;
  mesh.renderOrder = 1;
  return mesh;
}

/** A tapered limb from `a` to `b` with a smooth joint at `b`. */
function limb(a, b, radiusAtA, radiusAtB, material) {
  const group = new THREE.Group();
  const direction = new THREE.Vector3().subVectors(b, a);
  const length = direction.length();
  const mesh = shadowed(new THREE.Mesh(new THREE.CylinderGeometry(radiusAtB, radiusAtA, length, 14, 1, true), material));
  mesh.position.copy(a).addScaledVector(direction, 0.5);
  mesh.quaternion.setFromUnitVectors(UP, direction.normalize());
  group.add(mesh);
  const joint = shadowed(new THREE.Mesh(new THREE.SphereGeometry(radiusAtB, 14, 10), material));
  joint.position.copy(b);
  group.add(joint);
  return group;
}

/** A canopy clump: a few overlapping faceted blobs, dark underneath and lit on top. */
function canopy(center, radius, rand, dark = C.leafDark, light = C.leafLight) {
  const group = new THREE.Group();
  const material = facets();
  const count = 4;
  for (let i = 0; i < count; i++) {
    const r = radius * (0.55 + rand() * 0.35);
    const geometry = gradient(jitter(new THREE.IcosahedronGeometry(r, 2), r * 0.08, rand), dark, light, 0.8);
    const mesh = shadowed(new THREE.Mesh(geometry, material));
    const angle = (i / count) * Math.PI * 2 + rand();
    mesh.position.set(
      center.x + Math.cos(angle) * radius * 0.42,
      center.y + (i === 0 ? radius * 0.3 : (rand() - 0.5) * radius * 0.3),
      center.z + Math.sin(angle) * radius * 0.42
    );
    group.add(mesh);
  }
  group.userData.sway = rand() * Math.PI * 2;
  return group;
}

function buildIsland(rand, shadowTexture) {
  const island = new THREE.Group();
  const wobble = (angle) => Math.sin(angle * 3 + 0.6) * 0.035 + Math.sin(angle * 5 + 2) * 0.018;

  // Lathe profiles run bottom to top so faces point outward.
  const grassGeometry = organicLathe(
    [
      [4.5, -0.4],
      [4.58, -0.3],
      [4.52, -0.16],
      [4.3, -0.04],
      [3.9, 0],
      [0, 0],
    ],
    96,
    wobble
  );
  // Grass: light in the middle, a touch deeper toward the rim.
  const position = grassGeometry.attributes.position;
  const colors = new Float32Array(position.count * 3);
  const center = new THREE.Color(C.grassTop);
  const rim = new THREE.Color(C.grassEdge);
  const side = new THREE.Color(C.grassSide);
  const c = new THREE.Color();
  for (let i = 0; i < position.count; i++) {
    const r = Math.hypot(position.getX(i), position.getZ(i));
    const y = position.getY(i);
    if (y < -0.12) c.copy(rim).lerp(side, Math.min(1, (-y - 0.12) / 0.25));
    else c.copy(center).lerp(rim, Math.pow(Math.min(1, r / 4.6), 3));
    colors.set([c.r, c.g, c.b], i * 3);
  }
  grassGeometry.setAttribute("color", new THREE.BufferAttribute(colors, 3));
  island.add(shadowed(new THREE.Mesh(grassGeometry, clay(0xffffff, { vertexColors: true, roughness: 0.9 })), false, true));

  const soilGeometry = organicLathe(
    [
      [3.7, -1.52],
      [4.0, -1.38],
      [4.12, -1.08],
      [4.3, -0.98],
      [4.34, -0.7],
      [4.42, -0.62],
      [4.5, -0.38],
    ],
    96,
    wobble
  );
  island.add(shadowed(new THREE.Mesh(gradient(soilGeometry, C.soilDeep, C.soil, 0.7), clay(0xffffff, { vertexColors: true })), false, true));

  // Faceted rock underside, like a lump pulled out of the ground.
  const rockGeometry = jitter(
    new THREE.LatheGeometry(
      [
        [0.0, -3.38],
        [0.8, -3.25],
        [1.7, -2.9],
        [2.6, -2.45],
        [3.3, -1.95],
        [3.72, -1.5],
      ].map(([x, y]) => new THREE.Vector2(x, y)),
      16
    ),
    0.22,
    rand
  );
  island.add(shadowed(new THREE.Mesh(gradient(rockGeometry, 0x5d544f, C.rockLight, 0.6), facets()), false, true));

  // Stepping stones from the gardener toward the tree.
  const stoneMaterial = clay(C.stone, { roughness: 0.85 });
  for (const [x, z, r, rot] of [
    [-1.55, 1.1, 0.3, 0.2],
    [-0.95, 0.55, 0.26, 1.1],
    [-0.35, 1.25, 0.22, 2.2],
    [-2.1, 0.45, 0.24, 0.7],
  ]) {
    const stone = shadowed(new THREE.Mesh(new THREE.CylinderGeometry(r, r * 1.06, 0.07, 24), stoneMaterial), false, true);
    stone.scale.z = 0.78;
    stone.rotation.y = rot;
    stone.position.set(x, 0.02, z);
    island.add(stone);
  }

  // Shrubs.
  for (const [x, z, r] of [
    [3.3, -0.9, 0.62],
    [2.75, -1.75, 0.45],
    [-3.1, -1.7, 0.7],
    [-2.2, -2.75, 0.5],
    [1.0, 3.3, 0.38],
  ]) {
    island.add(canopy(new THREE.Vector3(x, r * 0.55, z), r, rand, C.shrubDark, C.shrubLight));
    const shadow = contactShadow(shadowTexture, r * 1.6, 0.8);
    shadow.position.set(x, 0.012, z);
    island.add(shadow);
  }

  // Flower clusters.
  const stem = clay(C.grassSide);
  for (const [x, z] of [
    [2.2, 1.9],
    [-3.4, 0.6],
    [0.2, 2.6],
    [3.2, 1.0],
    [-1.4, -2.9],
  ]) {
    for (let i = 0; i < 5; i++) {
      const flower = new THREE.Group();
      const h = 0.14 + rand() * 0.12;
      const s = new THREE.Mesh(new THREE.CylinderGeometry(0.01, 0.012, h, 5), stem);
      s.position.y = h / 2;
      const head = shadowed(
        new THREE.Mesh(new THREE.SphereGeometry(0.045, 10, 8), clay(C.flowers[Math.floor(rand() * C.flowers.length)], { roughness: 0.6 })),
        true,
        false
      );
      head.position.y = h;
      flower.add(s, head);
      flower.position.set(x + (rand() - 0.5) * 0.45, 0, z + (rand() - 0.5) * 0.45);
      island.add(flower);
    }
  }

  // Pebbles.
  const pebble = facets();
  for (const [x, z, r] of [
    [3.75, 0.2, 0.2],
    [3.55, 0.45, 0.12],
    [-3.7, -0.4, 0.17],
    [1.9, 2.9, 0.14],
  ]) {
    const geometry = gradient(jitter(new THREE.IcosahedronGeometry(r, 1), r * 0.25, rand), C.rock, C.rockLight);
    const mesh = shadowed(new THREE.Mesh(geometry, pebble));
    mesh.scale.y = 0.7;
    mesh.position.set(x, r * 0.35, z);
    island.add(mesh);
  }

  return island;
}

/** A dead, drooping branch with Prune's ribbon tag. The group's origin is where it meets the trunk. */
function buildDeadBranch(direction, rand) {
  const group = new THREE.Group();
  const wood = clay(C.dead, { transparent: true, roughness: 0.9 });
  const leaf = clay(C.deadLeaf, { transparent: true, side: THREE.DoubleSide });
  const ribbon = clay(C.ribbon, { transparent: true, roughness: 0.6 });
  const card = clay(C.tag, { transparent: true, roughness: 0.7 });
  const out = direction.clone().normalize();
  const a = new THREE.Vector3();
  const b = out.clone().multiplyScalar(1.25).add(new THREE.Vector3(0, 0.1, 0));
  const c = b.clone().add(out.clone().multiplyScalar(0.7)).add(new THREE.Vector3(0, -0.42, 0));
  const twig = b.clone().add(new THREE.Vector3(0, 0.36, 0)).addScaledVector(out, 0.18);
  group.add(limb(a, b, 0.13, 0.08, wood));
  group.add(limb(b, c, 0.08, 0.035, wood));
  group.add(limb(b, twig, 0.05, 0.022, wood));

  for (const point of [c, twig, b.clone().lerp(c, 0.5)]) {
    const l = shadowed(new THREE.Mesh(new THREE.SphereGeometry(0.09, 8, 6), leaf));
    l.scale.set(1, 0.25, 0.6);
    l.position.copy(point).add(new THREE.Vector3(0, -0.06, 0));
    l.rotation.set(rand() * 3, rand() * 3, rand() * 3);
    group.add(l);
  }

  // Ribbon band and a hanging tag that swings a little.
  const knot = a.clone().lerp(b, 0.55);
  const band = shadowed(new THREE.Mesh(new THREE.TorusGeometry(0.105, 0.035, 10, 24), ribbon));
  band.position.copy(knot);
  band.quaternion.setFromUnitVectors(new THREE.Vector3(0, 0, 1), b.clone().sub(a).normalize());
  group.add(band);
  const swing = new THREE.Group();
  swing.position.copy(knot).add(new THREE.Vector3(0, -0.08, 0));
  group.add(swing);
  const cord = new THREE.Mesh(new THREE.CylinderGeometry(0.008, 0.008, 0.26, 5), ribbon);
  cord.position.y = -0.13;
  swing.add(cord);
  const tagYaw = Math.atan2(out.x, out.z) + Math.PI / 2;
  const tag = shadowed(new THREE.Mesh(new RoundedBoxGeometry(0.22, 0.3, 0.025, 3, 0.04), card));
  tag.position.y = -0.4;
  tag.rotation.y = tagYaw;
  swing.add(tag);
  const eyelet = new THREE.Mesh(new THREE.TorusGeometry(0.022, 0.008, 6, 12), ribbon);
  eyelet.position.y = -0.28;
  eyelet.rotation.y = tagYaw;
  swing.add(eyelet);

  const anchor = new THREE.Object3D();
  anchor.position.copy(knot).add(new THREE.Vector3(0, -0.6, 0));
  group.add(anchor);
  const tip = new THREE.Object3D();
  tip.position.copy(b);
  group.add(tip);

  group.userData = { out, anchor, tip, swing, materials: [wood, leaf, ribbon, card] };
  return group;
}

function buildTree(rand, shadowTexture) {
  const tree = new THREE.Group();
  const bark = clay(C.trunk, { roughness: 0.85 });
  const v = (x, y, z) => new THREE.Vector3(x, y, z);

  // Flared base, then a gently bending trunk.
  tree.add(
    shadowed(
      new THREE.Mesh(
        new THREE.LatheGeometry(
          [
            [0.62, 0],
            [0.48, 0.08],
            [0.4, 0.25],
            [0.36, 0.45],
          ].map(([x, y]) => new THREE.Vector2(x, y)),
          20
        ),
        bark
      )
    )
  );
  tree.add(limb(v(0, 0.4, 0), v(0.1, 1.3, 0.02), 0.36, 0.3, bark));
  tree.add(limb(v(0.1, 1.3, 0.02), v(-0.04, 2.4, 0.05), 0.3, 0.22, bark));

  const live = [
    [v(0.1, 1.6, 0.02), v(1.2, 2.5, -0.35), 0.15, 1.0],
    [v(-0.04, 2.35, 0.05), v(-0.75, 3.25, 0.45), 0.13, 1.1],
    [v(-0.04, 2.35, 0.05), v(0.7, 3.35, -0.55), 0.13, 1.0],
    [v(0.05, 1.5, 0.02), v(-1.2, 2.35, -0.55), 0.14, 0.95],
    [v(-0.04, 2.4, 0.05), v(-0.05, 3.65, -0.1), 0.16, 1.25],
  ];
  const crowns = [];
  for (const [from, to, radius, size] of live) {
    tree.add(limb(from, to, radius, radius * 0.55, bark));
    const crown = canopy(to, size, rand);
    crowns.push(crown);
    tree.add(crown);
  }

  const deadSpots = [
    { at: v(0.12, 0.85, 0.05), dir: v(1, 0.12, -0.35) },
    { at: v(-0.1, 1.0, 0.08), dir: v(-1, 0.22, 0.4) },
    { at: v(0.08, 1.4, -0.05), dir: v(0.75, 0.3, -1) },
  ];
  const dead = deadSpots.map(({ at, dir }, index) => {
    const branch = buildDeadBranch(dir, rand);
    branch.position.copy(at);
    branch.userData.home = at.clone();
    branch.userData.info = DEAD_BRANCHES[index];
    tree.add(branch);
    return branch;
  });

  tree.add(contactShadow(shadowTexture, 1.5));
  tree.userData = { crowns, dead };
  return tree;
}

function buildGardener() {
  const gardener = new THREE.Group();
  const skin = clay(C.skin, { roughness: 0.65 });
  const ink = clay(0x2b2523, { roughness: 0.4 });

  for (const x of [-0.15, 0.15]) {
    const leg = shadowed(new THREE.Mesh(new THREE.CapsuleGeometry(0.1, 0.3, 6, 14), clay(C.pants)));
    leg.position.set(x, 0.36, 0);
    gardener.add(leg);
    const shoe = shadowed(new THREE.Mesh(new THREE.SphereGeometry(0.13, 18, 12), clay(C.shoes, { roughness: 0.6 })));
    shoe.scale.set(1, 0.58, 1.45);
    shoe.position.set(x, 0.07, 0.05);
    gardener.add(shoe);
  }

  const body = shadowed(
    new THREE.Mesh(
      new THREE.LatheGeometry(
        [
          [0.001, 0.55],
          [0.37, 0.56],
          [0.43, 0.72],
          [0.41, 0.98],
          [0.34, 1.24],
          [0.2, 1.37],
          [0.001, 1.4],
        ].map(([x, y]) => new THREE.Vector2(x, y)),
        32
      ),
      clay(C.shirt)
    )
  );
  gardener.add(body);
  const apronMaterial = clay(C.apron, { side: THREE.DoubleSide, roughness: 0.9 });
  const apron = shadowed(new THREE.Mesh(new THREE.CylinderGeometry(0.425, 0.445, 0.6, 32, 1, true, -0.85, 1.7), apronMaterial));
  apron.position.y = 0.86;
  gardener.add(apron);
  const bib = shadowed(new THREE.Mesh(new THREE.CylinderGeometry(0.36, 0.415, 0.3, 32, 1, true, -0.55, 1.1), apronMaterial));
  bib.position.y = 1.15;
  gardener.add(bib);
  const pocket = new THREE.Mesh(new THREE.CylinderGeometry(0.433, 0.44, 0.13, 24, 1, true, -0.3, 0.6), clay(0xcdbd9b, { side: THREE.DoubleSide }));
  pocket.position.y = 0.8;
  gardener.add(pocket);
  const neck = new THREE.Mesh(new THREE.CylinderGeometry(0.11, 0.13, 0.12, 14), skin);
  neck.position.y = 1.42;
  gardener.add(neck);

  const head = new THREE.Group();
  head.position.y = 1.74;
  head.add(shadowed(new THREE.Mesh(new THREE.SphereGeometry(0.4, 32, 24), skin)));
  for (const x of [-0.13, 0.13]) {
    const eye = new THREE.Mesh(new THREE.SphereGeometry(0.042, 12, 10), ink);
    eye.scale.z = 0.5;
    eye.position.set(x, 0.03, 0.37);
    head.add(eye);
    const cheek = new THREE.Mesh(new THREE.SphereGeometry(0.06, 12, 10), clay(C.cheek, { transparent: true, opacity: 0.75 }));
    cheek.scale.z = 0.35;
    cheek.position.set(x * 1.7, -0.09, 0.33);
    head.add(cheek);
  }
  const nose = new THREE.Mesh(new THREE.SphereGeometry(0.055, 14, 10), skin);
  nose.position.set(0, -0.04, 0.4);
  head.add(nose);
  const smile = new THREE.Mesh(new THREE.TorusGeometry(0.055, 0.011, 6, 16, Math.PI), ink);
  smile.rotation.z = Math.PI;
  smile.position.set(0, -0.13, 0.375);
  head.add(smile);

  const hat = new THREE.Group();
  hat.position.y = 0.2;
  hat.rotation.set(-0.1, 0, 0.06);
  const straw = clay(C.hat, { roughness: 0.85 });
  hat.add(
    shadowed(
      new THREE.Mesh(
        new THREE.LatheGeometry(
          [
            [0.3, 0.05],
            [0.6, 0.02],
            [0.83, -0.07],
            [0.86, -0.1],
            [0.6, -0.02],
            [0.3, 0.0],
            [0.3, 0.05],
          ].map(([x, y]) => new THREE.Vector2(x, y)),
          48
        ),
        straw
      )
    )
  );
  hat.add(
    shadowed(
      new THREE.Mesh(
        new THREE.LatheGeometry(
          [
            [0.37, 0.0],
            [0.355, 0.2],
            [0.31, 0.36],
            [0.2, 0.41],
            [0.001, 0.42],
          ].map(([x, y]) => new THREE.Vector2(x, y)),
          40
        ),
        straw
      )
    )
  );
  const band = new THREE.Mesh(new THREE.CylinderGeometry(0.367, 0.375, 0.08, 40, 1, true), clay(C.hatBand, { side: THREE.DoubleSide }));
  band.position.y = 0.06;
  hat.add(band);
  head.add(hat);
  gardener.add(head);

  const arm = (side) => {
    const shoulder = new THREE.Group();
    shoulder.position.set(side * 0.4, 1.26, 0);
    const sleeve = shadowed(new THREE.Mesh(new THREE.CapsuleGeometry(0.085, 0.36, 6, 12), clay(C.shirt)));
    sleeve.position.y = -0.24;
    shoulder.add(sleeve);
    const hand = shadowed(new THREE.Mesh(new THREE.SphereGeometry(0.095, 16, 12), skin));
    hand.position.y = -0.52;
    shoulder.add(hand);
    gardener.add(shoulder);
    return { shoulder, hand };
  };
  const pointing = arm(1);
  const holding = arm(-1);
  holding.shoulder.rotation.set(-0.95, 0, -0.22);
  const board = shadowed(new THREE.Mesh(new RoundedBoxGeometry(0.32, 0.42, 0.035, 3, 0.03), clay(C.clipboard, { roughness: 0.6 })));
  board.position.set(0, -0.6, 0.12);
  board.rotation.x = 0.95;
  holding.shoulder.add(board);
  const paper = new THREE.Mesh(new RoundedBoxGeometry(0.26, 0.32, 0.01, 2, 0.01), clay(C.paper));
  paper.position.set(0, -0.02, 0.022);
  board.add(paper);
  for (let i = 0; i < 3; i++) {
    const line = new THREE.Mesh(new THREE.BoxGeometry(i === 2 ? 0.1 : 0.16, 0.012, 0.004), clay(i === 0 ? C.ribbon : 0xb8b0a2));
    line.position.set(-0.02, 0.06 - i * 0.06, 0.03);
    board.add(line);
  }
  const clip = new THREE.Mesh(new RoundedBoxGeometry(0.1, 0.05, 0.04, 2, 0.012), clay(C.steel, { metalness: 0.6, roughness: 0.35 }));
  clip.position.set(0, 0.2, 0.03);
  board.add(clip);

  gardener.userData = { head, pointing, body };
  return gardener;
}

function buildRobot(glow) {
  const robot = new THREE.Group();
  const rig = new THREE.Group(); // tilts and squashes; `robot` only moves
  robot.add(rig);
  const shell = new THREE.MeshPhysicalMaterial({ color: C.robot, roughness: 0.32, clearcoat: 1, clearcoatRoughness: 0.18 });
  const visorMaterial = new THREE.MeshPhysicalMaterial({ color: C.visor, roughness: 0.15, clearcoat: 1, clearcoatRoughness: 0.05 });
  const lit = new THREE.MeshBasicMaterial({ color: C.glow });
  const amber = new THREE.MeshBasicMaterial({ color: C.amber });

  const body = shadowed(new THREE.Mesh(new THREE.SphereGeometry(0.3, 40, 28), shell));
  body.scale.set(1, 1.22, 1);
  rig.add(body);
  const seam = new THREE.Mesh(new THREE.TorusGeometry(0.297, 0.006, 6, 48), clay(0xc9cdd1));
  seam.rotation.x = Math.PI / 2;
  seam.position.y = 0.05;
  rig.add(seam);
  const badge = new THREE.Mesh(new THREE.CircleGeometry(0.045, 20), lit);
  badge.position.set(0, 0.1, 0.285);
  badge.rotation.x = -0.25;
  rig.add(badge);

  const head = new THREE.Group();
  head.position.y = 0.6;
  const skull = shadowed(new THREE.Mesh(new THREE.SphereGeometry(0.29, 40, 28), shell));
  skull.scale.set(1.16, 0.86, 1);
  head.add(skull);
  const visor = new THREE.Mesh(new THREE.SphereGeometry(0.293, 40, 20, Math.PI / 2 - 1.05, 2.1, 1.05, 1.0), visorMaterial);
  visor.scale.copy(skull.scale);
  head.add(visor);
  const eyes = [];
  for (const x of [-0.095, 0.095]) {
    const eye = new THREE.Mesh(new THREE.CapsuleGeometry(0.028, 0.035, 4, 10), lit);
    eye.position.set(x, 0.0, 0.275);
    head.add(eye);
    eyes.push(eye);
  }
  const stalk = new THREE.Mesh(new THREE.CylinderGeometry(0.008, 0.008, 0.12, 6), clay(0xb8bec4));
  stalk.position.y = 0.3;
  head.add(stalk);
  const bulb = new THREE.Mesh(new THREE.SphereGeometry(0.03, 12, 10), amber);
  bulb.position.y = 0.37;
  head.add(bulb);
  rig.add(head);

  const arm = (side) => {
    const shoulder = new THREE.Group();
    shoulder.position.set(side * 0.37, 0.12, 0);
    const forearm = shadowed(new THREE.Mesh(new THREE.CapsuleGeometry(0.055, 0.2, 6, 12), shell));
    forearm.position.y = -0.17;
    shoulder.add(forearm);
    rig.add(shoulder);
    return shoulder;
  };
  arm(-1).rotation.z = -0.35;
  const right = arm(1);

  // Shears: two slim blades on one pivot, rust-colored grips.
  const shears = new THREE.Group();
  shears.position.set(0, -0.34, 0.04);
  right.add(shears);
  const steel = new THREE.MeshStandardMaterial({ color: C.steel, metalness: 0.85, roughness: 0.25 });
  const grip = clay(C.ribbon, { roughness: 0.5 });
  const blades = [];
  for (const side of [-1, 1]) {
    const half = new THREE.Group();
    const blade = shadowed(new THREE.Mesh(new THREE.ConeGeometry(0.035, 0.36, 14), steel));
    blade.scale.z = 0.35;
    blade.position.y = -0.19;
    blade.rotation.x = Math.PI;
    half.add(blade);
    const handle = shadowed(new THREE.Mesh(new THREE.TorusGeometry(0.05, 0.016, 8, 20), grip));
    handle.position.y = 0.08;
    half.add(handle);
    half.userData.side = side;
    shears.add(half);
    blades.push(half);
  }
  const pin = new THREE.Mesh(new THREE.CylinderGeometry(0.022, 0.022, 0.05, 12), steel);
  pin.rotation.x = Math.PI / 2;
  shears.add(pin);

  // Hover glow under the body.
  const halo = new THREE.Mesh(
    new THREE.PlaneGeometry(0.7, 0.7),
    new THREE.MeshBasicMaterial({ map: glow, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending })
  );
  halo.rotation.x = -Math.PI / 2;
  halo.position.y = -0.4;
  rig.add(halo);
  const ring = new THREE.Mesh(new THREE.TorusGeometry(0.11, 0.018, 8, 28), lit);
  ring.rotation.x = Math.PI / 2;
  ring.position.y = -0.36;
  rig.add(ring);

  robot.userData = { rig, head, right, blades, bulb, eyes, halo };
  return robot;
}

const ease = {
  inOut: (t) => (t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2),
  out: (t) => 1 - Math.pow(1 - t, 3),
  outBack: (t) => 1 + 2.4 * Math.pow(t - 1, 3) + 1.4 * Math.pow(t - 1, 2),
};

/**
 * Mount the garden into `container`. Returns a cleanup function, or null when WebGL is
 * unavailable (the container keeps its fallback).
 */
export function mountGarden(container, options = {}) {
  const reducedMotion = options.reducedMotion ?? window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const small = container.clientWidth < 560;

  let renderer;
  try {
    renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, preserveDrawingBuffer: !!options.capture });
  } catch {
    return null;
  }
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.VSMShadowMap;
  renderer.toneMapping = THREE.NeutralToneMapping;
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.domElement.setAttribute("aria-hidden", "true");
  renderer.domElement.classList.add("garden-canvas");
  container.appendChild(renderer.domElement);

  const labelLayer = document.createElement("div");
  labelLayer.className = "garden-labels";
  labelLayer.setAttribute("aria-hidden", "true");
  container.appendChild(labelLayer);

  const scene = new THREE.Scene();
  const pmrem = new THREE.PMREMGenerator(renderer);
  scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
  scene.environmentIntensity = 0.55;

  const camera = new THREE.PerspectiveCamera(18, 1, 1, 200);
  const viewDirection = new THREE.Vector3(1, 0.74, 1).normalize();
  const lookTarget = new THREE.Vector3(0, 0.45, 0);

  scene.add(new THREE.HemisphereLight(0xfff6e8, 0x8a7560, 0.7));
  const sun = new THREE.DirectionalLight(0xffe6c4, 2.3);
  sun.position.set(5, 11, 4);
  sun.castShadow = true;
  sun.shadow.mapSize.set(small ? 1024 : 2048, small ? 1024 : 2048);
  Object.assign(sun.shadow.camera, { left: -6.5, right: 6.5, top: 6.5, bottom: -6.5, near: 1, far: 30 });
  sun.shadow.radius = 9;
  sun.shadow.blurSamples = 16;
  sun.shadow.bias = -0.0006;
  scene.add(sun);
  const fill = new THREE.DirectionalLight(0xcfe0ff, 0.45);
  fill.position.set(-7, 4, -3);
  scene.add(fill);

  const rand = seeded(11);
  const shadowTexture = radialTexture([
    [0, "rgba(40,30,20,0.55)"],
    [0.45, "rgba(40,30,20,0.22)"],
    [1, "rgba(40,30,20,0)"],
  ]);
  const world = new THREE.Group();
  scene.add(world);
  world.add(buildIsland(rand, shadowTexture));

  const tree = buildTree(rand, shadowTexture);
  tree.position.set(0.3, 0, -0.5);
  world.add(tree);

  const gardener = buildGardener();
  gardener.position.set(-2.35, 0, 1.85);
  gardener.rotation.y = 0.78;
  gardener.scale.setScalar(1.22);
  world.add(gardener);
  const gardenerShadow = contactShadow(shadowTexture, 0.75);
  gardenerShadow.position.set(-2.35, 0.013, 1.85);
  world.add(gardenerShadow);

  const robot = buildRobot(
    radialTexture([
      [0, "rgba(201,242,107,0.9)"],
      [0.35, "rgba(201,242,107,0.25)"],
      [1, "rgba(201,242,107,0)"],
    ])
  );
  const robotHome = new THREE.Vector3(2.65, 2.25, 2.25);
  robot.position.copy(robotHome);
  robot.scale.setScalar(1.15);
  world.add(robot);
  const robotShadow = contactShadow(shadowTexture, 0.55);
  world.add(robotShadow);

  const dead = tree.userData.dead;
  const labels = dead.map((branch) => {
    const el = document.createElement("span");
    el.className = "garden-tag";
    el.innerHTML = `<b>${branch.userData.info.label}</b><i>${branch.userData.info.note}</i>`;
    labelLayer.appendChild(el);
    return el;
  });

  // Leaves knocked loose by a snip.
  const leafMaterial = clay(C.deadLeaf, { transparent: true, side: THREE.DoubleSide });
  const leafGeometry = new THREE.SphereGeometry(0.06, 8, 6);
  const particles = [];

  const falling = [];
  const state = { index: 0, phase: "rest", t: 0, from: robotHome.clone(), to: robotHome.clone(), nod: 0 };
  const tmp = new THREE.Vector3();
  const tmp2 = new THREE.Vector3();
  const tmpQ = new THREE.Quaternion();
  const down = new THREE.Vector3(0, -1, 0);
  const one = new THREE.Vector3(1, 1, 1);
  const lastRobot = robot.position.clone();

  function resize() {
    const width = container.clientWidth;
    const height = container.clientHeight;
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    const distance = Math.max(28, 30.5 / camera.aspect);
    camera.position.copy(lookTarget).addScaledVector(viewDirection, distance);
    camera.lookAt(lookTarget);
    camera.updateProjectionMatrix();
  }

  function standbyFor(branch) {
    branch.userData.tip.getWorldPosition(tmp);
    world.worldToLocal(tmp);
    const out = branch.userData.out.clone().setY(0).normalize();
    return tmp.clone().addScaledVector(out, 1.0).add(new THREE.Vector3(0, -0.02, 0));
  }

  function aimGardener(target, dt) {
    const { pointing, head } = gardener.userData;
    pointing.shoulder.getWorldPosition(tmp2);
    const direction = tmp.copy(target).sub(tmp2).normalize();
    tmpQ.copy(gardener.quaternion).invert();
    direction.applyQuaternion(tmpQ);
    const goal = new THREE.Quaternion().setFromUnitVectors(down, direction);
    pointing.shoulder.quaternion.slerp(goal, Math.min(1, dt * 4));
    const yaw = Math.atan2(direction.x, direction.z) * 0.5;
    head.rotation.y += (yaw - head.rotation.y) * Math.min(1, dt * 3);
  }

  const restArm = new THREE.Quaternion().setFromEuler(new THREE.Euler(0, 0, 0.16));
  function lowerArm(dt) {
    const { pointing, head } = gardener.userData;
    pointing.shoulder.quaternion.slerp(restArm, Math.min(1, dt * 3));
    head.rotation.y += (0 - head.rotation.y) * Math.min(1, dt * 2);
  }

  function setShears(open) {
    for (const half of robot.userData.blades) half.rotation.z = half.userData.side * open;
  }

  function burst(at) {
    for (let i = 0; i < 9; i++) {
      const mesh = new THREE.Mesh(leafGeometry, leafMaterial.clone());
      mesh.scale.set(1, 0.25, 0.6);
      mesh.position.copy(at);
      scene.add(mesh);
      particles.push({
        mesh,
        velocity: new THREE.Vector3((Math.random() - 0.5) * 1.6, 0.6 + Math.random() * 0.9, (Math.random() - 0.5) * 1.6),
        spin: Math.random() * 6,
        phase: Math.random() * 6,
        life: 0,
      });
    }
  }

  function detach(branch, label) {
    branch.userData.tip.getWorldPosition(tmp);
    burst(tmp.clone());
    scene.attach(branch);
    const out = branch.userData.out;
    falling.push({
      branch,
      velocity: new THREE.Vector3(out.x * 0.5, 0.9, out.z * 0.5),
      spin: new THREE.Vector3((Math.random() - 0.5) * 2.4, (Math.random() - 0.5) * 1.6, (Math.random() - 0.5) * 2.4),
      rest: 0,
      bounced: 0,
    });
    label.classList.add("cut");
    branch.userData.cut = true;
    state.nod = 1;
  }

  function regrow() {
    for (const [i, branch] of dead.entries()) {
      tree.add(branch);
      branch.position.copy(branch.userData.home);
      branch.rotation.set(0, 0, 0);
      branch.scale.setScalar(0.001);
      branch.visible = true;
      branch.userData.cut = false;
      branch.userData.grow = 0;
      for (const material of branch.userData.materials) material.opacity = 1;
      labels[i].classList.remove("cut");
    }
    falling.length = 0;
  }

  const box = new THREE.Box3();
  function updateFalling(dt) {
    for (const item of falling) {
      const { branch } = item;
      if (!branch.visible) continue;
      if (item.rest > 0) {
        item.rest += dt;
        if (item.rest > 1.2) {
          const k = Math.min(1, (item.rest - 1.2) / 0.9);
          for (const material of branch.userData.materials) material.opacity = 1 - k;
          branch.position.y -= dt * 0.12;
          if (k === 1) branch.visible = false;
        }
        continue;
      }
      item.velocity.y -= 7 * dt;
      branch.position.addScaledVector(item.velocity, dt);
      branch.rotation.x += item.spin.x * dt;
      branch.rotation.y += item.spin.y * dt;
      branch.rotation.z += item.spin.z * dt;
      box.setFromObject(branch);
      if (box.min.y < 0.02 && item.velocity.y < 0) {
        branch.position.y += 0.02 - box.min.y;
        if (item.bounced < 1) {
          item.velocity.set(item.velocity.x * 0.35, -item.velocity.y * 0.25, item.velocity.z * 0.35);
          item.spin.multiplyScalar(0.25);
          item.bounced++;
        } else {
          item.rest = 0.0001;
        }
      }
    }
    for (let i = particles.length - 1; i >= 0; i--) {
      const p = particles[i];
      p.life += dt;
      p.velocity.y -= 2.2 * dt;
      p.velocity.multiplyScalar(1 - dt * 1.6);
      p.mesh.position.addScaledVector(p.velocity, dt);
      p.mesh.position.x += Math.sin(p.life * 5 + p.phase) * dt * 0.25;
      p.mesh.rotation.set(p.life * p.spin, p.phase, Math.sin(p.life * 4 + p.phase) * 0.8);
      if (p.mesh.position.y < 0.03) p.mesh.position.y = 0.03;
      p.mesh.material.opacity = Math.max(0, 1 - Math.max(0, p.life - 1.2) / 0.8);
      if (p.life > 2) {
        scene.remove(p.mesh);
        p.mesh.material.dispose();
        particles.splice(i, 1);
      }
    }
  }

  let elapsed = 0;
  function step(dt) {
    elapsed += dt;
    state.t += dt;
    const { rig, bulb, head: robotHead, right, eyes, halo } = robot.userData;

    // Ambient life.
    gardener.userData.body.scale.y = 1 + Math.sin(elapsed * 1.8) * 0.01;
    bulb.visible = Math.sin(elapsed * 3.2) > -0.3;
    halo.material.opacity = 0.75 + Math.sin(elapsed * 6) * 0.15;
    const blink = elapsed % 3.4 < 0.12 ? 0.15 : 1;
    for (const eye of eyes) eye.scale.y = blink;
    for (const crown of tree.userData.crowns) {
      crown.rotation.z = Math.sin(elapsed * 0.8 + crown.userData.sway) * 0.012;
      crown.rotation.x = Math.cos(elapsed * 0.7 + crown.userData.sway) * 0.008;
    }
    for (const branch of dead) {
      if (!branch.userData.cut) branch.userData.swing.rotation.z = Math.sin(elapsed * 1.6 + branch.userData.home.y * 3) * 0.12;
      if (branch.userData.grow !== undefined && branch.userData.grow < 1) {
        branch.userData.grow = Math.min(1, branch.userData.grow + dt / 1.1);
        branch.scale.setScalar(Math.max(0.001, ease.outBack(branch.userData.grow)));
      }
    }
    world.rotation.y = Math.sin(elapsed * 0.16) * 0.05;

    // The gardener nods when a branch comes down.
    if (state.nod > 0) {
      state.nod = Math.max(0, state.nod - dt * 1.2);
      gardener.userData.head.rotation.x = Math.sin((1 - state.nod) * Math.PI * 2) * 0.12 * state.nod;
    }

    const branch = dead[state.index];
    const bob = Math.sin(elapsed * 2.2) * 0.07;

    if (state.phase === "rest") {
      robot.position.lerp(tmp.copy(robotHome).setY(robotHome.y + bob), Math.min(1, dt * 2));
      lowerArm(dt);
      right.rotation.x += (0 - right.rotation.x) * Math.min(1, dt * 3);
      setShears(0.04);
      if (state.t > 1.4) Object.assign(state, { phase: "point", t: 0 });
    } else if (state.phase === "point") {
      branch.userData.anchor.getWorldPosition(tmp2);
      aimGardener(tmp2.clone(), dt);
      robot.position.y = robotHome.y + bob;
      if (state.t > 0.9) {
        Object.assign(state, { phase: "fly", t: 0 });
        state.from.copy(robot.position);
        state.to.copy(standbyFor(branch));
      }
    } else if (state.phase === "fly") {
      branch.userData.anchor.getWorldPosition(tmp2);
      aimGardener(tmp2.clone(), dt);
      const k = ease.inOut(Math.min(1, state.t / 1.5));
      robot.position.lerpVectors(state.from, state.to, k);
      robot.position.y += Math.sin(k * Math.PI) * 0.45;
      setShears(0.04 + ease.out(Math.min(1, state.t / 1.2)) * 0.42);
      right.rotation.x += (-1.15 - right.rotation.x) * Math.min(1, dt * 4);
      if (state.t > 1.6) Object.assign(state, { phase: "snip", t: 0 });
    } else if (state.phase === "snip") {
      const open = state.t < 0.3 ? 0.46 : Math.max(0, 0.46 - (state.t - 0.3) * 6);
      setShears(open);
      if (open === 0 && state.t < 0.5) rig.scale.set(1.06, 0.94, 1.06);
      if (state.t > 0.38 && !branch.userData.cut) detach(branch, labels[state.index]);
      if (state.t > 0.95) {
        Object.assign(state, { phase: "back", t: 0 });
        state.from.copy(robot.position);
      }
    } else if (state.phase === "back") {
      lowerArm(dt);
      const k = ease.inOut(Math.min(1, state.t / 1.3));
      robot.position.lerpVectors(state.from, robotHome, k);
      robot.position.y += Math.sin(k * Math.PI) * 0.3;
      setShears(0.04);
      if (state.t > 1.4) {
        state.index++;
        state.t = 0;
        if (state.index >= dead.length) Object.assign(state, { index: 0, phase: "done" });
        else state.phase = "point";
      }
    } else if (state.phase === "done") {
      robot.position.y = robotHome.y + bob;
      lowerArm(dt);
      if (state.t > 3) {
        regrow();
        Object.assign(state, { phase: "rest", t: 0 });
      }
    }
    rig.scale.lerp(one, Math.min(1, dt * 8));

    // Face the work while working, the viewer otherwise; bank into movement.
    const facing = new THREE.Vector3(-0.4, 0, 1);
    if (state.phase === "fly" || state.phase === "snip") {
      branch.userData.tip.getWorldPosition(tmp);
      world.worldToLocal(tmp);
      facing.copy(tmp).sub(robot.position).setY(0);
    }
    if (facing.lengthSq() > 1e-4) {
      let delta = Math.atan2(facing.x, facing.z) - robot.rotation.y;
      delta = Math.atan2(Math.sin(delta), Math.cos(delta));
      robot.rotation.y += delta * Math.min(1, dt * 4);
    }
    const velocity = tmp2.copy(robot.position).sub(lastRobot).divideScalar(Math.max(dt, 1e-3));
    lastRobot.copy(robot.position);
    velocity.applyQuaternion(tmpQ.copy(robot.quaternion).invert());
    rig.rotation.x += (THREE.MathUtils.clamp(velocity.z * 0.12, -0.3, 0.3) - rig.rotation.x) * Math.min(1, dt * 6);
    rig.rotation.z += (THREE.MathUtils.clamp(-velocity.x * 0.12, -0.3, 0.3) - rig.rotation.z) * Math.min(1, dt * 6);
    robotHead.rotation.z = Math.sin(elapsed * 1.2) * 0.05;

    // The robot's contact shadow follows it and softens with height.
    robotShadow.position.set(robot.position.x, 0.014, robot.position.z);
    const height = Math.max(0, robot.position.y);
    robotShadow.material.opacity = THREE.MathUtils.clamp(0.9 - height * 0.22, 0.15, 0.9);
    robotShadow.scale.setScalar(1 + height * 0.18);

    updateFalling(dt);
  }

  function placeLabels() {
    const width = container.clientWidth;
    const height = container.clientHeight;
    const working = ["point", "fly", "snip"].includes(state.phase);
    dead.forEach((branch, i) => {
      const label = labels[i];
      label.classList.toggle("active", working && i === state.index);
      if (!branch.visible) return;
      branch.userData.anchor.getWorldPosition(tmp);
      tmp.project(camera);
      const x = (tmp.x * 0.5 + 0.5) * width;
      const y = (-tmp.y * 0.5 + 0.5) * height;
      label.style.transform = `translate(${x.toFixed(1)}px, ${y.toFixed(1)}px) translate(-50%, 0)`;
    });
  }

  function render() {
    renderer.render(scene, camera);
    placeLabels();
  }

  resize();
  const observer = new ResizeObserver(() => {
    resize();
    render();
  });
  observer.observe(container);
  container.classList.add("garden-ready");

  // `startAt` fast-forwards the loop (seconds); previews use it for deterministic frames.
  const startAt = reducedMotion ? 2.7 : options.startAt ?? 0;
  for (let i = 0; i < Math.round(startAt * 60); i++) step(1 / 60);
  render();
  if (reducedMotion || options.paused) return () => observer.disconnect();

  const clock = new THREE.Clock();
  let visible = true;
  const io = new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting;
    if (visible) clock.getDelta();
  });
  io.observe(container);

  let frame = 0;
  function loop() {
    frame = requestAnimationFrame(loop);
    if (!visible || document.hidden) return;
    step(Math.min(clock.getDelta(), 1 / 20));
    render();
  }
  loop();

  return () => {
    cancelAnimationFrame(frame);
    observer.disconnect();
    io.disconnect();
    renderer.dispose();
    pmrem.dispose();
  };
}
