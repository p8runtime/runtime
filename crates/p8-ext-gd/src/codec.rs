//! Reading and writing the file formats, over pure-Rust codecs.

use crate::gd::{ALPHA_MAX, Img, Pal, alpha, argb, blue, green, quantize, red};
use image::ImageFormat;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Gif,
    Jpeg,
    Png,
    Webp,
    Bmp,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Gif => "GIF",
            Format::Jpeg => "JPEG",
            Format::Png => "PNG",
            Format::Webp => "WEBP",
            Format::Bmp => "BMP",
        }
    }
}

/// The format `data` starts as (gdImageCreateFromString's check).
pub fn sniff(data: &[u8]) -> Option<Format> {
    if data.starts_with(b"GIF8") {
        Some(Format::Gif)
    } else if data.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(Format::Jpeg)
    } else if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some(Format::Png)
    } else if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some(Format::Webp)
    } else if data.starts_with(b"BM") {
        Some(Format::Bmp)
    } else {
        None
    }
}

/// libpng alpha (0 transparent, 255 opaque) as gd's (127 transparent, 0 opaque).
fn gd_alpha(a: u8) -> i32 {
    ALPHA_MAX - i32::from(a >> 1)
}

/// gd's alpha as an 8-bit opaque-is-255 one.
pub fn png_alpha(a: i32) -> u8 {
    (255 - ((a << 1) + (a >> 6))) as u8
}

pub fn decode(format: Format, data: &[u8]) -> Option<Img> {
    match format {
        Format::Png => decode_png(data),
        Format::Gif => decode_gif(data),
        Format::Jpeg => {
            let d = image::load_from_memory_with_format(data, ImageFormat::Jpeg).ok()?;
            let mut im = from_rgba(d.width(), d.height(), &d.to_rgba8(), false);
            if let Some(res) = jfif_resolution(data) {
                im.res = res;
            }
            Some(im)
        }
        Format::Webp => {
            let d = image::load_from_memory_with_format(data, ImageFormat::WebP).ok()?;
            Some(from_rgba(d.width(), d.height(), &d.to_rgba8(), true))
        }
        Format::Bmp => decode_bmp_palette(data).or_else(|| {
            let d = image::load_from_memory_with_format(data, ImageFormat::Bmp).ok()?;
            Some(from_rgba(d.width(), d.height(), &d.to_rgba8(), false))
        }),
    }
}

/// The dots per inch a JFIF header gives.
fn jfif_resolution(data: &[u8]) -> Option<(i64, i64)> {
    let mut i = 2;
    while i + 4 <= data.len() && data[i] == 0xff {
        let marker = data[i + 1];
        let len = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
        if marker == 0xe0 && data.get(i + 4..i + 9) == Some(b"JFIF\0") && len >= 16 {
            let d = &data[i + 4..];
            let (units, x, y) = (d[7], i64::from(u16::from_be_bytes([d[8], d[9]])), i64::from(u16::from_be_bytes([d[10], d[11]])));
            return match units {
                1 => Some((x, y)),
                2 => Some(((x as f64 * 2.54 + 0.5) as i64, (y as f64 * 2.54 + 0.5) as i64)),
                _ => None,
            };
        }
        if marker == 0xda {
            break;
        }
        i += 2 + len;
    }
    None
}

/// An uncompressed 1, 4 or 8-bit BMP as a palette image.
fn decode_bmp_palette(d: &[u8]) -> Option<Img> {
    let u32le = |at: usize| d.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let u16le = |at: usize| d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let (offset, header) = (u32le(10)? as usize, u32le(14)? as usize);
    if header < 40 || u32le(30)? != 0 {
        return None;
    }
    let (w, h) = (u32le(18)? as i32, u32le(22)? as i32);
    let bpp = usize::from(u16le(28)?);
    if !matches!(bpp, 1 | 4 | 8) || w <= 0 || h == 0 {
        return None;
    }
    let used = u32le(46)? as usize;
    let colors = if used == 0 { 1 << bpp } else { used.min(256) };
    let (width, height) = (w as usize, h.unsigned_abs() as usize);
    let mut im = Img::new(w, height as i32, false);
    im.blend = false;
    for i in 0..colors {
        let e = d.get(14 + header + i * 4..14 + header + i * 4 + 4)?;
        im.pal.push(Pal { r: i32::from(e[2]), g: i32::from(e[1]), b: i32::from(e[0]), a: 0, open: false });
    }
    let row = (width * bpp).div_ceil(32) * 4;
    for y in 0..height {
        let sy = if h > 0 { height - 1 - y } else { y };
        let line = d.get(offset + sy * row..offset + sy * row + row)?;
        for x in 0..width {
            let bit = x * bpp;
            let v = (line[bit / 8] >> (8 - bpp - bit % 8)) & (((1u16 << bpp) - 1) as u8);
            im.px[y * width + x] = i32::from(v).min(colors as i32 - 1);
        }
    }
    Some(im)
}

fn from_rgba(w: u32, h: u32, rgba: &[u8], with_alpha: bool) -> Img {
    let mut im = Img::new(w as i32, h as i32, true);
    for (i, p) in rgba.chunks_exact(4).enumerate() {
        let a = if with_alpha { gd_alpha(p[3]) } else { 0 };
        im.px[i] = argb(a, i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
    }
    im
}

fn decode_png(data: &[u8]) -> Option<Img> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(data));
    dec.set_transformations(png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let depth = info.bit_depth as u8;
    let line = info.line_size;
    let ri = reader.info();
    let mut im;
    match info.color_type {
        png::ColorType::Indexed | png::ColorType::Grayscale => {
            im = Img::new(w as i32, h as i32, false);
            im.blend = false;
            if info.color_type == png::ColorType::Indexed {
                let plte = ri.palette.as_deref().unwrap_or(&[]);
                for e in plte.chunks_exact(3) {
                    im.pal.push(Pal { r: i32::from(e[0]), g: i32::from(e[1]), b: i32::from(e[2]), a: 0, open: false });
                }
                if let Some(t) = ri.trns.as_deref() {
                    for (i, a) in t.iter().enumerate() {
                        if let Some(p) = im.pal.get_mut(i) {
                            p.a = gd_alpha(*a);
                        }
                        if *a == 0 && im.transparent < 0 {
                            im.transparent = i as i32;
                        }
                    }
                }
            } else {
                let levels = 1usize << depth.min(8);
                for i in 0..levels {
                    let v = (i * 255 / (levels - 1)) as i32;
                    im.pal.push(Pal { r: v, g: v, b: v, a: 0, open: false });
                }
                if depth < 8 {
                    // gd maps a packed grey sample to the same palette index.
                }
            }
            for y in 0..h {
                let row = &buf[y * line..(y + 1) * line];
                for x in 0..w {
                    let v = match depth {
                        8 => row[x],
                        d => {
                            let per = 8 / d as usize;
                            let shift = 8 - d as usize * (x % per + 1);
                            (row[x / per] >> shift) & ((1u8 << d) - 1)
                        }
                    };
                    im.px[y * w + x] = i32::from(v);
                }
            }
        }
        png::ColorType::Rgb | png::ColorType::Rgba | png::ColorType::GrayscaleAlpha => {
            im = Img::new(w as i32, h as i32, true);
            if info.color_type == png::ColorType::Rgb
                && let Some(t) = ri.trns.as_deref()
                && t.len() >= 3
            {
                // A colour key: 16-bit samples (the low byte is the 8-bit value).
                let k = if t.len() >= 6 { [t[1], t[3], t[5]] } else { [t[0], t[1], t[2]] };
                im.transparent = argb(0, i32::from(k[0]), i32::from(k[1]), i32::from(k[2]));
            }
            let n = info.color_type.samples();
            for y in 0..h {
                for x in 0..w {
                    let s = &buf[y * line + x * n..];
                    let c = match info.color_type {
                        png::ColorType::Rgb => argb(0, i32::from(s[0]), i32::from(s[1]), i32::from(s[2])),
                        png::ColorType::Rgba => argb(gd_alpha(s[3]), i32::from(s[0]), i32::from(s[1]), i32::from(s[2])),
                        _ => argb(gd_alpha(s[1]), i32::from(s[0]), i32::from(s[0]), i32::from(s[0])),
                    };
                    im.px[y * w + x] = c;
                }
            }
        }
    }
    if let Some(d) = ri.pixel_dims
        && d.unit == png::Unit::Meter
        && d.xppu > 0
        && d.yppu > 0
    {
        im.res = ((f64::from(d.xppu) * 0.0254 + 0.5) as i64, (f64::from(d.yppu) * 0.0254 + 0.5) as i64);
    }
    Some(im)
}

fn decode_gif(data: &[u8]) -> Option<Img> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::Indexed);
    let mut dec = opts.read_info(data).ok()?;
    let (sw, sh) = (i32::from(dec.width()), i32::from(dec.height()));
    let global: Vec<u8> = dec.global_palette().map(<[u8]>::to_vec).unwrap_or_default();
    let frame = dec.read_next_frame().ok()??.clone();
    let pal_bytes = frame.palette.clone().unwrap_or(global);
    let mut im = Img::new(sw, sh, false);
    im.blend = false;
    for e in pal_bytes.chunks_exact(3) {
        im.pal.push(Pal { r: i32::from(e[0]), g: i32::from(e[1]), b: i32::from(e[2]), a: 0, open: false });
    }
    if im.pal.is_empty() {
        im.pal.push(Pal::default());
    }
    if let Some(t) = frame.transparent {
        im.transparent = i32::from(t);
    }
    let used = frame.buffer.iter().copied().max().map_or(0, |m| usize::from(m) + 1).max(im.transparent as usize + 1).max(1);
    // libgd drops the palette entries past the highest index in use.
    im.pal.truncate(used.min(im.pal.len()).max(1));
    let fill = if im.transparent >= 0 { im.transparent } else { 0 };
    im.px.iter_mut().for_each(|p| *p = fill);
    let (fw, fh, left, top) = (usize::from(frame.width), usize::from(frame.height), usize::from(frame.left), usize::from(frame.top));
    for y in 0..fh {
        for x in 0..fw {
            let (dx, dy) = (left + x, top + y);
            if dx < sw as usize && dy < sh as usize {
                let v = i32::from(*frame.buffer.get(y * fw + x)?);
                if (v as usize) < im.pal.len() {
                    im.px[dy * sw as usize + dx] = v;
                }
            }
        }
    }
    Some(im)
}

// ---- encoding ----

/// RGB bytes of the whole image (alpha dropped).
fn rgb_bytes(im: &Img) -> Vec<u8> {
    let mut out = Vec::with_capacity(im.px.len() * 3);
    for &p in &im.px {
        let c = if im.tc { p } else { im.pal_tc(p) };
        out.extend_from_slice(&[red(c) as u8, green(c) as u8, blue(c) as u8]);
    }
    out
}

pub fn encode_jpeg(im: &Img, quality: u8) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.max(1));
    if im.res.0 > 0 && im.res.1 > 0 {
        enc.set_pixel_density(image::codecs::jpeg::PixelDensity {
            density: (im.res.0.min(65535) as u16, im.res.1.min(65535) as u16),
            unit: image::codecs::jpeg::PixelDensityUnit::Inches,
        });
    }
    enc.encode(&rgb_bytes(im), im.w as u32, im.h as u32, image::ExtendedColorType::Rgb8).ok()?;
    Some(out)
}

pub fn encode_png(im: &Img, level: i32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, im.w as u32, im.h as u32);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(match level {
        0 => png::Compression::NoCompression,
        1..=3 => png::Compression::Fast,
        7..=9 => png::Compression::High,
        _ => png::Compression::Balanced,
    });
    if im.res.0 > 0 && im.res.1 > 0 {
        let ppm = |dpi: i64| (dpi as f64 / 0.0254 + 0.5) as u32;
        enc.set_pixel_dims(Some(png::PixelDimensions { xppu: ppm(im.res.0), yppu: ppm(im.res.1), unit: png::Unit::Meter }));
    }
    let data: Vec<u8> = if im.tc {
        if im.save_alpha {
            enc.set_color(png::ColorType::Rgba);
            im.px.iter().flat_map(|&c| [red(c) as u8, green(c) as u8, blue(c) as u8, png_alpha(alpha(c))]).collect()
        } else {
            enc.set_color(png::ColorType::Rgb);
            if im.transparent >= 0 {
                let t = im.transparent;
                enc.set_trns(vec![0, red(t) as u8, 0, green(t) as u8, 0, blue(t) as u8]);
            }
            rgb_bytes(im)
        }
    } else {
        enc.set_color(png::ColorType::Indexed);
        let n = im.pal.len().max(1);
        // Entries with alpha (or the transparent one) go first in their own
        // order, the rest after them in reverse (libgd's remap); an all-opaque
        // palette keeps its order.
        let clear = |i: usize| im.transparent == i as i32 || im.pal.get(i).is_some_and(|p| !p.open && p.a != 0);
        let remap = (0..n).any(clear);
        let mut mapping: Vec<usize> = (0..n).collect();
        if remap {
            let mut next = 0;
            for i in 0..n {
                if clear(i) {
                    mapping[i] = next;
                    next += 1;
                }
            }
            for i in (0..n).rev() {
                if !clear(i) {
                    mapping[i] = next;
                    next += 1;
                }
            }
        }
        let mut plte = vec![0u8; n * 3];
        let mut trns = vec![255u8; n];
        for i in 0..n {
            let p = im.pal.get(i).copied().unwrap_or_default();
            let at = mapping[i];
            plte[at * 3..at * 3 + 3].copy_from_slice(&[p.r as u8, p.g as u8, p.b as u8]);
            if im.transparent == i as i32 {
                trns[at] = 0;
            } else if p.a != 0 {
                trns[at] = png_alpha(p.a);
            }
        }
        enc.set_palette(plte);
        if remap {
            let keep = (0..n).filter(|&i| clear(i)).count();
            trns.truncate(keep);
            enc.set_trns(trns);
        }
        let depth = match n {
            0..=2 => 1usize,
            3..=4 => 2,
            5..=16 => 4,
            _ => 8,
        };
        enc.set_depth(match depth {
            1 => png::BitDepth::One,
            2 => png::BitDepth::Two,
            4 => png::BitDepth::Four,
            _ => png::BitDepth::Eight,
        });
        let (w, h) = (im.w as usize, im.h as usize);
        let row_bytes = (w * depth).div_ceil(8);
        let mut packed = vec![0u8; row_bytes * h];
        for y in 0..h {
            for x in 0..w {
                let v = mapping[im.px[y * w + x] as usize] as u8;
                let bit = x * depth;
                packed[y * row_bytes + bit / 8] |= v << (8 - depth - bit % 8);
            }
        }
        packed
    };
    let mut w = enc.write_header().ok()?;
    w.write_image_data(&data).ok()?;
    w.finish().ok()?;
    Some(out)
}

pub fn encode_gif(im: &Img) -> Option<Vec<u8>> {
    let (pal, idx, transparent): (Vec<Pal>, Vec<u8>, Option<u8>) = if im.tc {
        quantize(im, 256)
    } else {
        let t = if im.transparent >= 0 { Some(im.transparent as u8) } else { None };
        (im.pal.clone(), im.px.iter().map(|&p| p as u8).collect(), t)
    };
    let size = pal.len().max(2).next_power_of_two();
    let mut flat = Vec::with_capacity(size * 3);
    for i in 0..size {
        let p = pal.get(i).copied().unwrap_or_default();
        flat.extend_from_slice(&[p.r as u8, p.g as u8, p.b as u8]);
    }
    let mut out = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut out, im.w as u16, im.h as u16, &flat).ok()?;
        let frame = gif::Frame {
            width: im.w as u16,
            height: im.h as u16,
            transparent,
            interlaced: im.interlace,
            buffer: std::borrow::Cow::Borrowed(&idx),
            ..gif::Frame::default()
        };
        enc.write_frame(&frame).ok()?;
    }
    Some(out)
}

/// Lossless only: no pure-Rust lossy WebP encoder exists.
pub fn encode_webp(im: &Img) -> Option<Vec<u8>> {
    let mut rgba = Vec::with_capacity(im.px.len() * 4);
    for &c in &im.px {
        rgba.extend_from_slice(&[red(c) as u8, green(c) as u8, blue(c) as u8, png_alpha(alpha(c))]);
    }
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out).encode(&rgba, im.w as u32, im.h as u32, image::ExtendedColorType::Rgba8).ok()?;
    Some(out)
}

/// libgd's BMP: 24-bit for truecolor, 8-bit with a palette otherwise,
/// uncompressed.
pub fn encode_bmp(im: &Img) -> Vec<u8> {
    let (w, h) = (im.w as usize, im.h as usize);
    let bpp = if im.tc { 24 } else { 8 };
    let row = if im.tc { (w * 3).div_ceil(4) * 4 } else { w.div_ceil(4) * 4 };
    let colors = im.pal.len().max(1);
    let pal_len = if im.tc { 0 } else { colors * 4 };
    let offset = 14 + 40 + pal_len;
    let size = offset + row * h;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(bpp as u16).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&((row * h) as u32).to_le_bytes());
    let ppm = |dpi: i64| (dpi as f64 / 0.0254 + 0.5) as i32;
    out.extend_from_slice(&ppm(im.res.0).to_le_bytes());
    out.extend_from_slice(&ppm(im.res.1).to_le_bytes());
    out.extend_from_slice(&(if im.tc { 0u32 } else { colors as u32 }).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    if !im.tc {
        for i in 0..colors {
            let p = im.pal.get(i).copied().unwrap_or_default();
            out.extend_from_slice(&[p.b as u8, p.g as u8, p.r as u8, 0]);
        }
    }
    for y in (0..h).rev() {
        let start = out.len();
        for x in 0..w {
            let p = im.px[y * w + x];
            if im.tc {
                out.extend_from_slice(&[blue(p) as u8, green(p) as u8, red(p) as u8]);
            } else {
                out.push(p as u8);
            }
        }
        while out.len() - start < row {
            out.push(0);
        }
    }
    out
}
