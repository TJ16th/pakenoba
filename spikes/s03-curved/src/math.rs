//! 最小限のベクトルと変換。

pub type V3 = [f32; 3];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}

pub fn normalize(a: V3) -> V3 {
    let len = length(a);
    if len <= f32::EPSILON { [0.0, 0.0, 0.0] } else { scale(a, 1.0 / len) }
}

/// 軸 `axis`（単位ベクトル）まわりに `v` を `angle` ラジアン回す（ロドリゲスの公式）。
pub fn rotate(v: V3, axis: V3, angle: f32) -> V3 {
    let (s, c) = angle.sin_cos();
    add(add(scale(v, c), scale(cross(axis, v), s)), scale(axis, dot(axis, v) * (1.0 - c)))
}

/// 回転と平行移動。`p' = m * p + t`。
#[derive(Clone, Copy, Debug)]
pub struct Affine {
    pub m: [V3; 3],
    pub t: V3,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], t: [0.0, 0.0, 0.0] };

    /// 点 `origin` を通る軸 `axis`（単位ベクトル）まわりの回転。
    pub fn rotation_about(origin: V3, axis: V3, angle: f32) -> Affine {
        // 列ごとに基底を回して行列を作る。
        let cols = [rotate([1.0, 0.0, 0.0], axis, angle), rotate([0.0, 1.0, 0.0], axis, angle), rotate([0.0, 0.0, 1.0], axis, angle)];
        let m = [[cols[0][0], cols[1][0], cols[2][0]], [cols[0][1], cols[1][1], cols[2][1]], [cols[0][2], cols[1][2], cols[2][2]]];
        let rotated_origin = [dot(m[0], origin), dot(m[1], origin), dot(m[2], origin)];
        Affine { m, t: sub(origin, rotated_origin) }
    }

    pub fn point(&self, p: V3) -> V3 {
        add(self.vector(p), self.t)
    }

    pub fn vector(&self, v: V3) -> V3 {
        [dot(self.m[0], v), dot(self.m[1], v), dot(self.m[2], v)]
    }

    /// `self` を後に適用する合成。`(self.then_after(inner))(p) = self(inner(p))`。
    pub fn then_after(&self, inner: &Affine) -> Affine {
        let mut m = [[0.0f32; 3]; 3];
        for (r, row) in m.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = self.m[r][0] * inner.m[0][c] + self.m[r][1] * inner.m[1][c] + self.m[r][2] * inner.m[2][c];
            }
        }
        Affine { m, t: self.point(inner.t) }
    }
}

/// 4x4 行列（列優先。WGSL の `mat4x4<f32>` と同じ並び）。
pub type Mat4 = [[f32; 4]; 4];

pub fn mat4_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0f32; 4]; 4];
    for (c, col) in out.iter_mut().enumerate() {
        for (r, cell) in col.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

/// 右手系の視点行列。
pub fn look_at(eye: V3, target: V3, up: V3) -> Mat4 {
    let f = normalize(sub(target, eye));
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    [[s[0], u[0], -f[0], 0.0], [s[1], u[1], -f[1], 0.0], [s[2], u[2], -f[2], 0.0], [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0]]
}

/// 右手系の透視投影。奥行きは 0〜1（wgpu の規約）。
pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y * 0.5).tan();
    [[f / aspect, 0.0, 0.0, 0.0], [0.0, f, 0.0, 0.0], [0.0, 0.0, far / (near - far), -1.0], [0.0, 0.0, near * far / (near - far), 0.0]]
}
