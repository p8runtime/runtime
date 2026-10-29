//! The image a GdImage holds and what libgd does to it: pixels (palette
//! indexes or 0xAARRGGBB with a 7-bit alpha, 0 opaque), alpha blending,
//! drawing, copying and resampling.
//!
//! Reference: $PHP_SRC_PATH/ext/gd/libgd/gd.c, gd_interpolation.c, gd_rotate.c,
//! gd_crop.c

pub const ALPHA_MAX: i32 = 127;
pub const MAX_COLORS: usize = 256;

pub const fn argb(a: i32, r: i32, g: i32, b: i32) -> i32 {
    (a << 24) | (r << 16) | (g << 8) | b
}
pub const fn alpha(c: i32) -> i32 {
    (c >> 24) & 0x7f
}
pub const fn red(c: i32) -> i32 {
    (c >> 16) & 0xff
}
pub const fn green(c: i32) -> i32 {
    (c >> 8) & 0xff
}
pub const fn blue(c: i32) -> i32 {
    c & 0xff
}

/// One palette entry.
#[derive(Clone, Copy, Default, Debug)]
pub struct Pal {
    pub r: i32,
    pub g: i32,
    pub b: i32,
    pub a: i32,
    pub open: bool,
}

#[derive(Clone)]
pub struct Img {
    pub w: i32,
    pub h: i32,
    pub tc: bool,
    /// Truecolor: 0xAARRGGBB; palette: the index.
    pub px: Vec<i32>,
    pub pal: Vec<Pal>,
    pub transparent: i32,
    pub blend: bool,
    pub save_alpha: bool,
    pub interlace: bool,
    pub interp: i64,
    pub res: (i64, i64),
    pub thick: i32,
    pub clip: (i32, i32, i32, i32),
}

impl Img {
    pub fn new(w: i32, h: i32, tc: bool) -> Img {
        Img {
            w,
            h,
            tc,
            px: vec![0; (w as usize) * (h as usize)],
            pal: Vec::new(),
            transparent: -1,
            blend: tc,
            save_alpha: false,
            interlace: false,
            interp: 3,
            res: (96, 96),
            thick: 1,
            clip: (0, 0, w - 1, h - 1),
        }
    }

    pub fn safe(&self, x: i32, y: i32) -> bool {
        x >= self.clip.0 && x <= self.clip.2 && y >= self.clip.1 && y <= self.clip.3
    }

    fn at(&self, x: i32, y: i32) -> usize {
        (y as usize) * (self.w as usize) + x as usize
    }

    /// gdImageGetPixel: the index or the truecolor; 0 outside.
    pub fn get(&self, x: i32, y: i32) -> i32 {
        if self.safe(x, y) { self.px[self.at(x, y)] } else { 0 }
    }

    /// gdImageGetTrueColorPixel.
    pub fn get_tc(&self, x: i32, y: i32) -> i32 {
        if !self.safe(x, y) {
            return 0;
        }
        let p = self.px[self.at(x, y)];
        if self.tc { p } else { self.pal_tc(p) }
    }

    pub fn pal_tc(&self, p: i32) -> i32 {
        match self.pal.get(p as usize) {
            Some(e) => argb(if self.transparent == p { ALPHA_MAX } else { e.a }, e.r, e.g, e.b),
            None => 0,
        }
    }

    pub fn set_pixel(&mut self, x: i32, y: i32, color: i32) {
        if !self.safe(x, y) {
            return;
        }
        let i = self.at(x, y);
        if self.tc {
            self.px[i] = if self.blend { alpha_blend(self.px[i], color) } else { color };
        } else if color >= 0 && (color as usize) < self.pal.len() {
            self.px[i] = color;
        }
    }

    /// gdImageColorAllocateAlpha: -1 when the palette is full.
    pub fn allocate(&mut self, r: i32, g: i32, b: i32, a: i32) -> i32 {
        if self.tc {
            return argb(a, r, g, b);
        }
        let slot = match self.pal.iter().position(|p| p.open) {
            Some(i) => i,
            None => {
                if self.pal.len() >= MAX_COLORS {
                    return -1;
                }
                self.pal.push(Pal::default());
                self.pal.len() - 1
            }
        };
        self.pal[slot] = Pal { r, g, b, a, open: false };
        slot as i32
    }

    pub fn closest(&self, r: i32, g: i32, b: i32, a: i32) -> i32 {
        if self.tc {
            return argb(a, r, g, b);
        }
        let mut best = -1;
        let mut min = i64::MAX;
        for (i, p) in self.pal.iter().enumerate() {
            if p.open {
                continue;
            }
            let d = i64::from((p.r - r).pow(2) + (p.g - g).pow(2) + (p.b - b).pow(2) + (p.a - a).pow(2));
            if d < min {
                min = d;
                best = i as i32;
            }
        }
        best
    }

    pub fn exact(&self, r: i32, g: i32, b: i32, a: i32) -> i32 {
        if self.tc {
            return argb(a, r, g, b);
        }
        self.pal.iter().position(|p| !p.open && p.r == r && p.g == g && p.b == b && p.a == a).map_or(-1, |i| i as i32)
    }

    /// gdImageColorResolveAlpha: the exact colour, else a new one, else the closest.
    pub fn resolve(&mut self, r: i32, g: i32, b: i32, a: i32) -> i32 {
        if self.tc {
            return argb(a, r, g, b);
        }
        let e = self.exact(r, g, b, a);
        if e != -1 {
            return e;
        }
        let n = self.allocate(r, g, b, a);
        if n != -1 { n } else { self.closest(r, g, b, a) }
    }

    /// The image as truecolor, in place (gdImagePaletteToTrueColor): the
    /// transparent colour's pixels become fully transparent black.
    pub fn to_truecolor(&mut self) {
        if self.tc {
            return;
        }
        let t = self.transparent;
        let px: Vec<i32> = self.px.iter().map(|&p| if p == t { argb(ALPHA_MAX, 0, 0, 0) } else { self.pal_tc(p) }).collect();
        if t >= 0
            && let Some(e) = self.pal.get(t as usize)
        {
            self.transparent = argb(ALPHA_MAX, e.r, e.g, e.b);
        }
        self.px = px;
        self.tc = true;
        self.pal.clear();
        self.blend = true;
    }

    /// The truecolor pixel as its channels, the palette's colour for an index.
    pub fn channels(&self, c: i32) -> (i32, i32, i32, i32) {
        let c = if self.tc { c } else { self.pal_tc(c) };
        (red(c), green(c), blue(c), alpha(c))
    }
}

/// gdAlphaBlend: `src` over `dst`.
pub fn alpha_blend(dst: i32, src: i32) -> i32 {
    let sa = alpha(src);
    if sa == 0 {
        return src;
    }
    let da = alpha(dst);
    if sa == ALPHA_MAX {
        return dst;
    }
    if da == ALPHA_MAX {
        return src;
    }
    let sw = ALPHA_MAX - sa;
    let dw = (ALPHA_MAX - da) * sa / ALPHA_MAX;
    let tw = sw + dw;
    let a = sa * da / ALPHA_MAX;
    let r = (red(src) * sw + red(dst) * dw) / tw;
    let g = (green(src) * sw + green(dst) * dw) / tw;
    let b = (blue(src) * sw + blue(dst) * dw) / tw;
    argb(a, r, g, b)
}

// ---- drawing ----

pub fn line(im: &mut Img, x1: i32, y1: i32, x2: i32, y2: i32, color: i32) {
    let thick = im.thick.max(1);
    let (dx, dy) = ((x2 - x1).abs(), (y2 - y1).abs());
    if thick > 1 {
        // Offsets across the line's thin axis, as gd does for vertical and horizontal runs.
        let (wid, vertical) = if dy <= dx { (thick, true) } else { (thick, false) };
        let half = wid / 2;
        for k in 0..wid {
            let o = k - half;
            if vertical {
                thin_line(im, x1, y1 + o, x2, y2 + o, color);
            } else {
                thin_line(im, x1 + o, y1, x2 + o, y2, color);
            }
        }
        return;
    }
    thin_line(im, x1, y1, x2, y2, color);
}

fn thin_line(im: &mut Img, mut x1: i32, mut y1: i32, x2: i32, y2: i32, color: i32) {
    let dx = (x2 - x1).abs();
    let dy = (y2 - y1).abs();
    let sx = if x1 < x2 { 1 } else { -1 };
    let sy = if y1 < y2 { 1 } else { -1 };
    let mut err = dx - dy;
    loop {
        im.set_pixel(x1, y1, color);
        if x1 == x2 && y1 == y2 {
            break;
        }
        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            x1 += sx;
        }
        if e2 < dx {
            err += dx;
            y1 += sy;
        }
    }
}

pub fn rectangle(im: &mut Img, mut x1: i32, mut y1: i32, mut x2: i32, mut y2: i32, color: i32) {
    if x1 > x2 {
        std::mem::swap(&mut x1, &mut x2);
    }
    if y1 > y2 {
        std::mem::swap(&mut y1, &mut y2);
    }
    let t = im.thick.max(1);
    if t > 1 {
        let half = t / 2;
        let (a, b) = (half, t - half - 1);
        let (x1o, y1o, x2o, y2o) = (x1 - a, y1 - a, x2 + b, y2 + b);
        let (x1i, y1i, x2i, y2i) = (x1 + b, y1 + b, x2 - a, y2 - a);
        filled_rectangle(im, x1o, y1o, x2o, y1i, color);
        filled_rectangle(im, x1o, y2i, x2o, y2o, color);
        filled_rectangle(im, x1o, y1i + 1, x1i, y2i - 1, color);
        filled_rectangle(im, x2i, y1i + 1, x2o, y2i - 1, color);
        return;
    }
    thin_line(im, x1, y1, x2, y1, color);
    if y2 > y1 {
        thin_line(im, x1, y2, x2, y2, color);
    }
    if y2 - y1 > 1 {
        thin_line(im, x1, y1 + 1, x1, y2 - 1, color);
        if x2 > x1 {
            thin_line(im, x2, y1 + 1, x2, y2 - 1, color);
        }
    }
}

pub fn filled_rectangle(im: &mut Img, mut x1: i32, mut y1: i32, mut x2: i32, mut y2: i32, color: i32) {
    if x1 > x2 {
        std::mem::swap(&mut x1, &mut x2);
    }
    if y1 > y2 {
        std::mem::swap(&mut y1, &mut y2);
    }
    let x1 = x1.max(im.clip.0);
    let x2 = x2.min(im.clip.2);
    let y1 = y1.max(im.clip.1);
    let y2 = y2.min(im.clip.3);
    for y in y1..=y2 {
        for x in x1..=x2 {
            im.set_pixel(x, y, color);
        }
    }
}

/// Fill the region of the colour at (x, y) with `color` (gdImageFill).
pub fn fill(im: &mut Img, x: i32, y: i32, color: i32) {
    if !im.safe(x, y) {
        return;
    }
    let old = im.get(x, y);
    let blend = std::mem::replace(&mut im.blend, false);
    if old != color {
        let mut stack = vec![(x, y)];
        while let Some((x, y)) = stack.pop() {
            if !im.safe(x, y) || im.get(x, y) != old {
                continue;
            }
            let mut l = x;
            while im.safe(l - 1, y) && im.get(l - 1, y) == old {
                l -= 1;
            }
            let mut r = x;
            while im.safe(r + 1, y) && im.get(r + 1, y) == old {
                r += 1;
            }
            for i in l..=r {
                im.set_pixel(i, y, color);
            }
            for ny in [y - 1, y + 1] {
                let mut i = l;
                while i <= r {
                    if im.safe(i, ny) && im.get(i, ny) == old {
                        stack.push((i, ny));
                        while i <= r && im.safe(i, ny) && im.get(i, ny) == old {
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
        }
    }
    im.blend = blend;
}

pub fn fill_to_border(im: &mut Img, x: i32, y: i32, border: i32, color: i32) {
    if !im.safe(x, y) {
        return;
    }
    let blend = std::mem::replace(&mut im.blend, false);
    let mut stack = vec![(x, y)];
    while let Some((x, y)) = stack.pop() {
        if !im.safe(x, y) {
            continue;
        }
        let p = im.get(x, y);
        if p == border || p == color {
            continue;
        }
        im.set_pixel(x, y, color);
        stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
    }
    im.blend = blend;
}

/// An ellipse centred on (cx, cy) with full width `w` and height `h`.
pub fn ellipse(im: &mut Img, cx: i32, cy: i32, w: i32, h: i32, color: i32, filled: bool) {
    let (a, b) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    if a <= 0.0 || b <= 0.0 {
        return;
    }
    let (ai, bi) = (a as i32, b as i32);
    if filled {
        for dy in -bi..=bi {
            let t = 1.0 - (f64::from(dy) / b).powi(2);
            if t < 0.0 {
                continue;
            }
            let dx = (a * t.sqrt()).round() as i32;
            for x in (cx - dx)..=(cx + dx) {
                im.set_pixel(x, cy + dy, color);
            }
        }
        return;
    }
    let steps = ((a + b) * 8.0) as i32 + 8;
    let mut prev: Option<(i32, i32)> = None;
    for i in 0..=steps {
        let t = f64::from(i) * std::f64::consts::TAU / f64::from(steps);
        let p = (cx + (f64::from(ai) * t.cos()).round() as i32, cy + (f64::from(bi) * t.sin()).round() as i32);
        if let Some(q) = prev {
            thin_line(im, q.0, q.1, p.0, p.1, color);
        }
        prev = Some(p);
    }
}

pub fn polygon(im: &mut Img, pts: &[(i32, i32)], color: i32, close: bool) {
    for w in pts.windows(2) {
        line(im, w[0].0, w[0].1, w[1].0, w[1].1, color);
    }
    if close && pts.len() > 2 {
        let (a, b) = (pts[pts.len() - 1], pts[0]);
        line(im, a.0, a.1, b.0, b.1, color);
    }
}

pub fn filled_polygon(im: &mut Img, pts: &[(i32, i32)], color: i32) {
    if pts.is_empty() {
        return;
    }
    let (min_y, max_y) = pts.iter().fold((i32::MAX, i32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
    for y in min_y.max(im.clip.1)..=max_y.min(im.clip.3) {
        let mut xs = Vec::new();
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            if (a.1 <= y && b.1 > y) || (b.1 <= y && a.1 > y) {
                let t = f64::from(y - a.1) / f64::from(b.1 - a.1);
                xs.push((f64::from(a.0) + t * f64::from(b.0 - a.0)).round() as i32);
            }
        }
        xs.sort_unstable();
        for pair in xs.chunks(2) {
            if let [l, r] = pair {
                for x in *l..=*r {
                    im.set_pixel(x, y, color);
                }
            }
        }
    }
}

// ---- copying ----

/// gdImageCopy.
pub fn copy(dst: &mut Img, src: &Img, dx: i32, dy: i32, sx: i32, sy: i32, w: i32, h: i32) {
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (sx + x, sy + y);
            if dst.tc {
                let c = src.get_tc(px, py);
                if c != src.transparent {
                    dst.set_pixel(dx + x, dy + y, c);
                }
            } else {
                let c = src.get(px, py);
                if src.transparent == c && !src.tc {
                    continue;
                }
                let (r, g, b, a) = src.channels(c);
                let n = dst.resolve(r, g, b, a);
                dst.set_pixel(dx + x, dy + y, n);
            }
        }
    }
}

/// gdImageCopyMerge (`gray`: gdImageCopyMergeGray).
pub fn copy_merge(dst: &mut Img, src: &Img, dx: i32, dy: i32, sx: i32, sy: i32, w: i32, h: i32, pct: i32, gray: bool) {
    for y in 0..h {
        for x in 0..w {
            let c = src.get(sx + x, sy + y);
            if !src.tc && src.transparent == c {
                continue;
            }
            let d = dst.get(dx + x, dy + y);
            let (sr, sg, sb, sa) = src.channels(c);
            let (mut dr, mut dg, mut db, _) = dst.channels(d);
            if gray {
                let g = (0.29900 * f64::from(dr) + 0.58700 * f64::from(dg) + 0.11400 * f64::from(db)) as i32;
                dr = g;
                dg = g;
                db = g;
            }
            let p = f64::from(pct) / 100.0;
            let q = f64::from(100 - pct) / 100.0;
            let nr = (f64::from(sr) * p + f64::from(dr) * q) as i32;
            let ng = (f64::from(sg) * p + f64::from(dg) * q) as i32;
            let nb = (f64::from(sb) * p + f64::from(db) * q) as i32;
            let n = dst.resolve(nr, ng, nb, sa);
            dst.set_pixel(dx + x, dy + y, n);
        }
    }
}

/// How many destination pixels each of `s` source pixels covers when
/// stretched to `d` (gd accumulates the ratio, so a source pixel can cover none).
fn spans(d: i32, s: i32) -> Vec<i32> {
    let mut acc = 0.0f32;
    (0..s)
        .map(|_| {
            let before = acc;
            acc += d as f32 / s as f32;
            (acc as i32) - (before as i32)
        })
        .collect()
}

/// gdImageCopyResized: each source pixel is repeated over the destination
/// pixels it covers (or dropped).
pub fn copy_resized(dst: &mut Img, src: &Img, dx: i32, dy: i32, sx: i32, sy: i32, dw: i32, dh: i32, sw: i32, sh: i32) {
    if dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0 {
        return;
    }
    let stx = spans(dw, sw);
    let sty = spans(dh, sh);
    let mut toy = dy;
    for y in sy..sy + sh {
        let rows = sty[(y - sy) as usize];
        for _ in 0..rows {
            let mut tox = dx;
            for x in sx..sx + sw {
                let n = stx[(x - sx) as usize];
                if n == 0 {
                    continue;
                }
                let raw = src.get(x, y);
                if !src.tc && src.transparent == raw {
                    tox += n;
                    continue;
                }
                let color = if dst.tc {
                    src.get_tc(x, y)
                } else {
                    let (r, g, b, a) = src.channels(raw);
                    dst.resolve(r, g, b, a)
                };
                for _ in 0..n {
                    dst.set_pixel(tox, toy, color);
                    tox += 1;
                }
            }
            toy += 1;
        }
    }
}

/// gdImageCopyResampled: the area average of the source pixels under each
/// destination pixel, weighted by their alpha.
pub fn copy_resampled(dst: &mut Img, src: &Img, dx: i32, dy: i32, sx: i32, sy: i32, dw: i32, dh: i32, sw: i32, sh: i32) {
    if !dst.tc {
        return copy_resized(dst, src, dx, dy, sx, sy, dw, dh, sw, sh);
    }
    if dw <= 0 || dh <= 0 {
        return;
    }
    for y in dy..dy + dh {
        let sy1 = f64::from(y - dy) * f64::from(sh) / f64::from(dh);
        let sy2 = f64::from(y + 1 - dy) * f64::from(sh) / f64::from(dh);
        for x in dx..dx + dw {
            let sx1 = f64::from(x - dx) * f64::from(sw) / f64::from(dw);
            let sx2 = f64::from(x + 1 - dx) * f64::from(sw) / f64::from(dw);
            let (mut r, mut g, mut b, mut a) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let (mut alpha_sum, mut contrib_sum) = (0.0f64, 0.0f64);
            let mut cy = sy1;
            loop {
                let yportion = if cy.floor() == sy1.floor() {
                    let p = 1.0 - (cy - cy.floor());
                    cy = cy.floor();
                    p.min(sy2 - sy1)
                } else if cy == sy2.floor() {
                    sy2 - sy2.floor()
                } else {
                    1.0
                };
                let mut cx = sx1;
                loop {
                    let xportion = if cx.floor() == sx1.floor() {
                        let p = 1.0 - (cx - cx.floor());
                        cx = cx.floor();
                        p.min(sx2 - sx1)
                    } else if cx == sx2.floor() {
                        sx2 - sx2.floor()
                    } else {
                        1.0
                    };
                    let contribution = xportion * yportion;
                    let p = src.get_tc(cx as i32 + sx, cy as i32 + sy);
                    let factor = f64::from(ALPHA_MAX - alpha(p)) * contribution;
                    r += f64::from(red(p)) * factor;
                    g += f64::from(green(p)) * factor;
                    b += f64::from(blue(p)) * factor;
                    a += f64::from(alpha(p)) * contribution;
                    alpha_sum += factor;
                    contrib_sum += contribution;
                    cx += 1.0;
                    if cx >= sx2 {
                        break;
                    }
                }
                cy += 1.0;
                if cy >= sy2 {
                    break;
                }
            }
            if contrib_sum != 0.0 {
                a /= contrib_sum;
            }
            if alpha_sum != 0.0 {
                r /= alpha_sum;
                g /= alpha_sum;
                b /= alpha_sum;
            } else {
                r = 0.0;
                g = 0.0;
                b = 0.0;
            }
            let clamp = |v: f64, hi: f64| if v + 0.5 > hi { hi } else { v + 0.5 };
            let c = argb(clamp(a, 127.0) as i32, clamp(r, 255.0) as i32, clamp(g, 255.0) as i32, clamp(b, 255.0) as i32);
            dst.set_pixel(x, y, c);
        }
    }
}

// ---- new images from old ones ----

/// A truecolor copy of `im`'s pixels (palette converted).
pub fn truecolor_copy(im: &Img) -> Img {
    let mut t = im.clone();
    t.to_truecolor();
    t
}

/// gdImageCrop of the rectangle; area outside the image is transparent.
pub fn crop(im: &Img, x: i32, y: i32, w: i32, h: i32) -> Img {
    let mut out = if im.tc {
        let mut o = Img::new(w, h, true);
        o.save_alpha = true;
        o.blend = false;
        o
    } else {
        let mut o = Img::new(w, h, false);
        o.pal = im.pal.clone();
        o.transparent = im.transparent;
        o
    };
    let blank = 0;
    for yy in 0..h {
        for xx in 0..w {
            let (px, py) = (x + xx, y + yy);
            let v = if px >= 0 && py >= 0 && px < im.w && py < im.h { im.px[im.at(px, py)] } else { blank };
            let i = out.at(xx, yy);
            out.px[i] = v;
        }
    }
    if im.tc {
        out.blend = true;
    }
    out.res = im.res;
    out
}

pub fn flip(im: &mut Img, horizontal: bool, vertical: bool) {
    let (w, h) = (im.w as usize, im.h as usize);
    if horizontal {
        for row in im.px.chunks_mut(w) {
            row.reverse();
        }
    }
    if vertical {
        for y in 0..h / 2 {
            for x in 0..w {
                im.px.swap(y * w + x, (h - 1 - y) * w + x);
            }
        }
    }
}

/// gdImageRotate90/180/270 (counter-clockwise quarter turns).
pub fn rotate_quarter(im: &Img, quarters: i32) -> Img {
    let (w, h) = (im.w, im.h);
    let (nw, nh) = if quarters % 2 == 0 { (w, h) } else { (h, w) };
    let mut out = im.clone();
    out.w = nw;
    out.h = nh;
    out.clip = (0, 0, nw - 1, nh - 1);
    out.px = vec![0; (nw as usize) * (nh as usize)];
    for y in 0..h {
        for x in 0..w {
            let v = im.px[im.at(x, y)];
            let (nx, ny) = match quarters {
                1 => (y, w - 1 - x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (h - 1 - y, x),
            };
            let i = out.at(nx, ny);
            out.px[i] = v;
        }
    }
    out
}

/// gdImageRotateInterpolated: any angle, bilinear, on a truecolor result.
pub fn rotate(src: &Img, degrees: f64, bg: i32) -> Img {
    let mut s = src.clone();
    s.to_truecolor();
    let angle = degrees.rem_euclid(360.0);
    let turns = angle.round() as i32;
    if (angle - f64::from(turns)).abs() < 1e-9 && turns % 90 == 0 {
        let mut out = match turns {
            0 => s.clone(),
            90 => rotate_quarter(&s, 1),
            180 => rotate_quarter(&s, 2),
            _ => rotate_quarter(&s, 3),
        };
        out.save_alpha = true;
        return out;
    }
    let rad = degrees.to_radians();
    let (sin, cos) = rad.sin_cos();
    let (sw, sh) = (s.w, s.h);
    // libgd's bounding box: the width comes out one less than the height's formula.
    let nw = ((f64::from(sw) * cos.abs() + f64::from(sh) * sin.abs()).floor() as i32 - 1).max(1);
    let nh = ((f64::from(sw) * sin.abs() + f64::from(sh) * cos.abs()).floor() as i32).max(1);
    let mut out = Img::new(nw, nh, true);
    out.save_alpha = true;
    out.blend = false;
    out.res = s.res;
    // gdImageRotateBilinear: 24.8 fixed point, the source position of each
    // destination pixel relative to the centres; the outermost source
    // rows and columns count as outside.
    let fx = |v: f64| (v * 256.0) as i64;
    let (f_sin, f_cos) = (fx(rad.sin()), fx(rad.cos()));
    let (f_h, f_w) = (i64::from(sh / 2) << 8, i64::from(sw / 2) << 8);
    let mul = |a: i64, b: i64| (a * b) >> 8;
    for i in 0..nh {
        for j in 0..nw {
            let f_i = i64::from(i - nh / 2) << 8;
            let f_j = i64::from(j - nw / 2) << 8;
            let f_m = mul(f_j, f_sin) + mul(f_i, f_cos) + 384 + f_h;
            let f_n = mul(f_j, f_cos) - mul(f_i, f_sin) + 384 + f_w;
            let (m, n) = (f_m >> 8, f_n >> 8);
            let c = if m >= 0 && m < i64::from(sh) && n >= 0 && n < i64::from(sw) {
                let f = f_m - (m << 8);
                let g = f_n - (n << 8);
                let w = [mul(256 - f, 256 - g), mul(256 - f, g), mul(f, 256 - g), mul(f, g)];
                let at = |x: i64, y: i64| s.get_tc(x.min(i64::from(sw) - 1) as i32, y.min(i64::from(sh) - 1) as i32);
                let p = [at(n, m), at(n + 1, m), at(n, m + 1), at(n + 1, m + 1)];
                let mix = |f: fn(i32) -> i32| ((0..4).map(|k| i64::from(f(p[k])) * w[k]).sum::<i64>() >> 8) as i32;
                argb(mix(alpha).clamp(0, 127), mix(red).clamp(0, 255), mix(green).clamp(0, 255), mix(blue).clamp(0, 255))
            } else {
                bg
            };
            let idx = out.at(j, i);
            out.px[idx] = c;
        }
    }
    out.blend = true;
    out
}

/// gdImageScaleBilinear: 24.8 fixed-point bilinear sampling at the
/// destination pixel's position in the source (edges clamped).
fn scale_bilinear(s: &Img, out: &mut Img) {
    let (nw, nh) = (out.w, out.h);
    let step_x = ((s.w as f32 / nw as f32) * 256.0) as i64;
    let step_y = ((s.h as f32 / nh as f32) * 256.0) as i64;
    // A sample past the right or bottom edge is the base pixel itself.
    let get = |x: i64, y: i64, base: (i64, i64)| {
        let (x, y) = if x >= i64::from(s.w) || y >= i64::from(s.h) { base } else { (x, y) };
        s.get_tc(x as i32, y as i32)
    };
    for i in 0..nh {
        let py = i64::from(i) * step_y;
        let (m, fa) = (py >> 8, py & 255);
        for j in 0..nw {
            let px = i64::from(j) * step_x;
            let (n, fb) = (px >> 8, px & 255);
            let w1 = ((256 - fa) * (256 - fb)) >> 8;
            let w2 = ((256 - fa) * fb) >> 8;
            let w3 = (fa * (256 - fb)) >> 8;
            let w4 = (fa * fb) >> 8;
            let b = (n, m);
            let p = [get(n, m, b), get(n + 1, m, b), get(n, m + 1, b), get(n + 1, m + 1, b)];
            let mix = |f: fn(i32) -> i32| {
                let sum = i64::from(f(p[0])) * w1 + i64::from(f(p[1])) * w2 + i64::from(f(p[2])) * w3 + i64::from(f(p[3])) * w4;
                (sum >> 8) as i32
            };
            let idx = (i as usize) * (nw as usize) + j as usize;
            out.px[idx] = argb(mix(alpha).min(127), mix(red).min(255), mix(green).min(255), mix(blue).min(255));
        }
    }
}

/// gdImageScale: a two-pass triangle filter to a truecolor image.
pub fn scale(src: &Img, nw: i32, nh: i32, nearest: bool, bilinear_fixed: bool) -> Img {
    let mut s = src.clone();
    s.to_truecolor();
    let mut out = Img::new(nw, nh, true);
    out.blend = false;
    out.res = s.res;
    if nearest {
        for y in 0..nh {
            for x in 0..nw {
                let (sx, sy) = ((i64::from(x) * i64::from(s.w) / i64::from(nw)) as i32, (i64::from(y) * i64::from(s.h) / i64::from(nh)) as i32);
                let i = out.at(x, y);
                out.px[i] = s.get_tc(sx, sy);
            }
        }
        out.blend = true;
        return out;
    }
    if nw == s.w && nh == s.h {
        out.px = s.px.clone();
        out.blend = true;
        return out;
    }
    if bilinear_fixed {
        scale_bilinear(&s, &mut out);
        out.blend = true;
        return out;
    }
    // Horizontal pass into an intermediate (nw x s.h), then vertical.
    let mid = filter_pass(&s.px, s.w as usize, s.h as usize, nw as usize, true);
    let fin = filter_pass(&mid, nw as usize, s.h as usize, nh as usize, false);
    out.px = fin;
    out.blend = true;
    out
}

fn filter_pass(px: &[i32], w: usize, h: usize, new: usize, horizontal: bool) -> Vec<i32> {
    let (src_len, lines) = if horizontal { (w, h) } else { (h, w) };
    let scale = new as f64 / src_len as f64;
    let (radius, sc) = if scale < 1.0 { (1.0 / scale, scale) } else { (1.0, 1.0) };
    let mut weights: Vec<(usize, Vec<f64>)> = Vec::with_capacity(new);
    for u in 0..new {
        let center = (u as f64 + 0.5) / scale - 0.5;
        let left = ((center - radius).floor().max(0.0)) as usize;
        let right = (((center + radius).ceil()) as i64).min(src_len as i64 - 1).max(0) as usize;
        let mut ws: Vec<f64> = (left..=right).map(|i| (1.0 - ((i as f64 - center) * sc).abs()).max(0.0)).collect();
        let total: f64 = ws.iter().sum();
        if total > 0.0 {
            ws.iter_mut().for_each(|w| *w /= total);
        } else {
            let near = (center.round().max(0.0) as usize).min(src_len - 1);
            return filter_nearest(px, w, h, new, horizontal, near);
        }
        weights.push((left, ws));
    }
    let (out_w, out_h) = if horizontal { (new, h) } else { (w, new) };
    let mut out = vec![0i32; out_w * out_h];
    for line in 0..lines {
        for (u, (left, ws)) in weights.iter().enumerate() {
            let (mut a, mut r, mut g, mut b, mut asum) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for (k, wt) in ws.iter().enumerate() {
                let i = left + k;
                let p = if horizontal { px[line * w + i] } else { px[i * w + line] };
                let af = f64::from(ALPHA_MAX - alpha(p)) * wt;
                a += f64::from(alpha(p)) * wt;
                r += f64::from(red(p)) * af;
                g += f64::from(green(p)) * af;
                b += f64::from(blue(p)) * af;
                asum += af;
            }
            if asum > 0.0 {
                r /= asum;
                g /= asum;
                b /= asum;
            }
            let c = argb((a + 0.5).min(127.0) as i32, (r + 0.5).min(255.0) as i32, (g + 0.5).min(255.0) as i32, (b + 0.5).min(255.0) as i32);
            if horizontal {
                out[line * out_w + u] = c;
            } else {
                out[u * out_w + line] = c;
            }
        }
    }
    out
}

fn filter_nearest(px: &[i32], w: usize, h: usize, new: usize, horizontal: bool, _near: usize) -> Vec<i32> {
    let (out_w, out_h) = if horizontal { (new, h) } else { (w, new) };
    let mut out = vec![0i32; out_w * out_h];
    for y in 0..out_h {
        for x in 0..out_w {
            let (sx, sy) = if horizontal { (x * w / new, y) } else { (x, y * h / new) };
            out[y * out_w + x] = px[sy * w + sx];
        }
    }
    out
}

// ---- colour reduction ----

/// The distinct colours of a truecolor image, or a NeuQuant palette of at
/// most `max`: (palette, index per pixel, the transparent index).
pub fn quantize(im: &Img, max: usize) -> (Vec<Pal>, Vec<u8>, Option<u8>) {
    let t = truecolor_copy(im);
    let tcol = t.transparent;
    let transparent = |c: i32| tcol >= 0 && c == tcol;
    let mut colors: Vec<i32> = Vec::new();
    let mut exact = true;
    let mut index = std::collections::HashMap::new();
    let has_transparent = t.px.iter().any(|&c| transparent(c));
    let limit = if has_transparent { max.saturating_sub(1) } else { max };
    for &c in &t.px {
        if transparent(c) {
            continue;
        }
        let key = c & 0x00ff_ffff;
        if let std::collections::hash_map::Entry::Vacant(e) = index.entry(key) {
            if colors.len() >= limit {
                exact = false;
                break;
            }
            e.insert(colors.len() as u8);
            colors.push(key);
        }
    }
    let mut pal: Vec<Pal> = Vec::new();
    let mut idx = vec![0u8; t.px.len()];
    let trans_index;
    if exact {
        for &k in &colors {
            pal.push(Pal { r: red(k), g: green(k), b: blue(k), a: 0, open: false });
        }
        trans_index = if has_transparent {
            pal.push(Pal { r: 0, g: 0, b: 0, a: 0, open: false });
            Some((pal.len() - 1) as u8)
        } else {
            None
        };
        for (i, &c) in t.px.iter().enumerate() {
            idx[i] = if transparent(c) { trans_index.unwrap_or(0) } else { index[&(c & 0x00ff_ffff)] };
        }
    } else {
        let mut samples = Vec::with_capacity(t.px.len() * 4);
        for &c in t.px.iter().filter(|&&c| !transparent(c)) {
            samples.extend_from_slice(&[red(c) as u8, green(c) as u8, blue(c) as u8, 255]);
        }
        let nq = color_quant::NeuQuant::new(10, limit.max(2), &samples);
        for e in nq.color_map_rgb().chunks(3) {
            pal.push(Pal { r: i32::from(e[0]), g: i32::from(e[1]), b: i32::from(e[2]), a: 0, open: false });
        }
        trans_index = if has_transparent {
            pal.push(Pal { r: 0, g: 0, b: 0, a: 0, open: false });
            Some((pal.len() - 1) as u8)
        } else {
            None
        };
        for (i, &c) in t.px.iter().enumerate() {
            idx[i] = if transparent(c) { trans_index.unwrap_or(0) } else { nq.index_of(&[red(c) as u8, green(c) as u8, blue(c) as u8, 255]) as u8 };
        }
    }
    (pal, idx, trans_index)
}
