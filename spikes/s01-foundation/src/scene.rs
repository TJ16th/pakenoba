//! 箱の形とカメラ。座標は箱の中心が原点、x が幅（右）、y が高さ（上）、z が奥行き（手前が正）。

pub type V3 = [f32; 3];

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn normalize(a: V3) -> V3 {
    let len = dot(a, a).sqrt();
    if len <= f32::EPSILON { [0.0, 0.0, 0.0] } else { [a[0] / len, a[1] / len, a[2] / len] }
}

/// 箱の外寸（mm）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxDims {
    pub w: f32,
    pub h: f32,
    pub d: f32,
}

impl Default for BoxDims {
    fn default() -> Self {
        BoxDims { w: 120.0, h: 160.0, d: 60.0 }
    }
}

/// 箱の周りを回るカメラ。角度はラジアン。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { yaw: 0.6, pitch: 0.35, zoom: 1.0 }
    }
}

/// 箱の1面。角は外から見て 左上、右上、右下、左下 の順。
pub struct Face {
    pub corners: [V3; 4],
    pub normal: V3,
    /// 絵柄を貼る場合の横方向の範囲（0〜1）。None は紙の地色のまま。
    pub tex_u: Option<(f32, f32)>,
}

/// 6面を返す。絵柄は正面から右側面へ続く1枚の帯として貼る（面またぎの確認用）。
pub fn faces(dims: BoxDims) -> [Face; 6] {
    let (x, y, z) = (dims.w * 0.5, dims.h * 0.5, dims.d * 0.5);
    let split = dims.w / (dims.w + dims.d);
    [
        // 正面
        Face { corners: [[-x, y, z], [x, y, z], [x, -y, z], [-x, -y, z]], normal: [0.0, 0.0, 1.0], tex_u: Some((0.0, split)) },
        // 右側面
        Face { corners: [[x, y, z], [x, y, -z], [x, -y, -z], [x, -y, z]], normal: [1.0, 0.0, 0.0], tex_u: Some((split, 1.0)) },
        // 背面
        Face { corners: [[x, y, -z], [-x, y, -z], [-x, -y, -z], [x, -y, -z]], normal: [0.0, 0.0, -1.0], tex_u: None },
        // 左側面
        Face { corners: [[-x, y, -z], [-x, y, z], [-x, -y, z], [-x, -y, -z]], normal: [-1.0, 0.0, 0.0], tex_u: None },
        // 天面
        Face { corners: [[-x, y, -z], [x, y, -z], [x, y, z], [-x, y, z]], normal: [0.0, 1.0, 0.0], tex_u: None },
        // 底面
        Face { corners: [[-x, -y, z], [x, -y, z], [x, -y, -z], [-x, -y, -z]], normal: [0.0, -1.0, 0.0], tex_u: None },
    ]
}

/// 画面への投影。
pub struct Projector {
    eye: V3,
    right: V3,
    up: V3,
    forward: V3,
    focal: f32,
    cx: f32,
    cy: f32,
}

/// 投影後の点。`inv_z` は奥行きの逆数（遠近補正に使う）。
#[derive(Clone, Copy)]
pub struct Projected {
    pub x: f32,
    pub y: f32,
    pub inv_z: f32,
}

impl Projector {
    pub fn new(dims: BoxDims, cam: Camera, width: f32, height: f32) -> Self {
        let fov = 30f32.to_radians();
        let radius = 0.5 * (dims.w * dims.w + dims.h * dims.h + dims.d * dims.d).sqrt();
        let pitch = cam.pitch.clamp(-1.45, 1.45);
        let zoom = cam.zoom.clamp(0.3, 4.0);
        let dist = radius / (fov * 0.5).sin() * 1.2 / zoom;
        let dir = [pitch.cos() * cam.yaw.sin(), pitch.sin(), pitch.cos() * cam.yaw.cos()];
        let eye = [dir[0] * dist, dir[1] * dist, dir[2] * dist];
        let forward = normalize([-dir[0], -dir[1], -dir[2]]);
        let right = normalize(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        // 縦横の短いほうに箱が収まるようにする。
        let focal = 0.5 * width.min(height) / (fov * 0.5).tan();
        Projector { eye, right, up, forward, focal, cx: width * 0.5, cy: height * 0.5 }
    }

    pub fn eye(&self) -> V3 {
        self.eye
    }

    pub fn project(&self, p: V3) -> Projected {
        let rel = sub(p, self.eye);
        let z = dot(rel, self.forward).max(1e-3);
        Projected { x: self.cx + dot(rel, self.right) / z * self.focal, y: self.cy - dot(rel, self.up) / z * self.focal, inv_z: 1.0 / z }
    }
}
