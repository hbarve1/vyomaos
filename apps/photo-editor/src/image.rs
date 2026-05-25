// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

#[derive(Clone)]
pub struct Image {
    pub w:    usize,
    pub h:    usize,
    pub data: Vec<u8>, // RGB triples
}

impl Image {
    pub fn new(w: usize, h: usize, r: u8, g: u8, b: u8) -> Self {
        let mut data = Vec::with_capacity(w * h * 3);
        for _ in 0..w * h {
            data.push(r);
            data.push(g);
            data.push(b);
        }
        Image { w, h, data }
    }

    pub fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let i = (y * self.w + x) * 3;
        (self.data[i], self.data[i + 1], self.data[i + 2])
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, r: u8, g: u8, b: u8) {
        let i = (y * self.w + x) * 3;
        self.data[i] = r;
        self.data[i + 1] = g;
        self.data[i + 2] = b;
    }

    pub fn crop(&self, cx: usize, cy: usize, cw: usize, ch: usize) -> Self {
        let cw = cw.min(self.w.saturating_sub(cx));
        let ch = ch.min(self.h.saturating_sub(cy));
        if cw == 0 || ch == 0 { return self.clone(); }
        let mut out = Image::new(cw, ch, 0, 0, 0);
        for y in 0..ch {
            for x in 0..cw {
                let (r, g, b) = self.pixel(cx + x, cy + y);
                out.set_pixel(x, y, r, g, b);
            }
        }
        out
    }

    pub fn resize(&self, nw: usize, nh: usize) -> Self {
        if nw == 0 || nh == 0 { return self.clone(); }
        let mut out = Image::new(nw, nh, 0, 0, 0);
        for y in 0..nh {
            for x in 0..nw {
                let sx = x * self.w / nw;
                let sy = y * self.h / nh;
                let (r, g, b) = self.pixel(sx.min(self.w - 1), sy.min(self.h - 1));
                out.set_pixel(x, y, r, g, b);
            }
        }
        out
    }

    pub fn rotate90(&self) -> Self {
        let mut out = Image::new(self.h, self.w, 0, 0, 0);
        for y in 0..self.h {
            for x in 0..self.w {
                let (r, g, b) = self.pixel(x, y);
                out.set_pixel(self.h - 1 - y, x, r, g, b);
            }
        }
        out
    }

    pub fn brightness(&mut self, delta: i32) {
        for v in self.data.iter_mut() {
            *v = (*v as i32 + delta).clamp(0, 255) as u8;
        }
    }

    pub fn contrast(&mut self, delta: i32) {
        let factor = (259 * (delta + 255)) / (255 * (259 - delta));
        for v in self.data.iter_mut() {
            *v = (factor * (*v as i32 - 128) / 256 + 128).clamp(0, 255) as u8;
        }
    }

    pub fn save_ppm(&self, path: &str) -> Result<(), String> {
        let mut out: Vec<u8> = Vec::new();
        let header = format!("P6\n{} {}\n255\n", self.w, self.h);
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(&self.data);
        std::fs::write(path, out).map_err(|e| e.to_string())
    }

    pub fn load_ppm(path: &str) -> Result<Self, String> {
        let raw = std::fs::read(path).map_err(|e| e.to_string())?;
        let mut pos = 0usize;

        let skip_comment_ws = |data: &[u8], p: &mut usize| {
            while *p < data.len() {
                if data[*p] == b'#' {
                    while *p < data.len() && data[*p] != b'\n' { *p += 1; }
                } else if data[*p].is_ascii_whitespace() {
                    *p += 1;
                } else {
                    break;
                }
            }
        };

        let read_token = |data: &[u8], p: &mut usize| -> String {
            let mut tok = String::new();
            while *p < data.len() && !data[*p].is_ascii_whitespace() {
                tok.push(data[*p] as char);
                *p += 1;
            }
            tok
        };

        skip_comment_ws(&raw, &mut pos);
        let magic = read_token(&raw, &mut pos);
        if magic != "P6" { return Err("not P6".to_string()); }
        skip_comment_ws(&raw, &mut pos);
        let ws = read_token(&raw, &mut pos).parse::<usize>().map_err(|e| e.to_string())?;
        skip_comment_ws(&raw, &mut pos);
        let hs = read_token(&raw, &mut pos).parse::<usize>().map_err(|e| e.to_string())?;
        skip_comment_ws(&raw, &mut pos);
        let _maxval = read_token(&raw, &mut pos);
        if pos < raw.len() && raw[pos].is_ascii_whitespace() { pos += 1; }
        let expected = ws * hs * 3;
        if raw.len() - pos < expected {
            return Err(format!("truncated: need {} got {}", expected, raw.len() - pos));
        }
        let data = raw[pos..pos + expected].to_vec();
        Ok(Image { w: ws, h: hs, data })
    }
}

pub fn make_demo_image() -> Image {
    let w = 120usize;
    let h = 80usize;
    let mut img = Image::new(w, h, 0, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let r = (x * 255 / w) as u8;
            let g = (y * 255 / h) as u8;
            let b = ((x + y) * 128 / (w + h)) as u8;
            img.set_pixel(x, y, r, g, b);
        }
    }
    img
}
