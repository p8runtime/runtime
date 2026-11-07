#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's gd extension: the GdImage class and the image* functions, over
//! pure-Rust codecs (png, zune-jpeg, gif, image-webp, the image crate's
//! JPEG/BMP writers): no libgd. What is observable follows libgd as
//! php-src bundles it: dimensions, colours, resampling, return values,
//! warnings. Encoded bytes differ from libgd's, and WebP is written lossless
//! (no pure-Rust lossy encoder exists). Text (FreeType and the built-in
//! fonts), arcs, brushes/tiles/styles, WBMP/XBM/GD/GD2/TGA/AVIF and the affine
//! functions are not implemented.
//!
//! Reference: $PHP_SRC_PATH/ext/gd/gd.c, ext/gd/libgd

mod codec;
mod exif;
mod exif_tags;
mod gd;

use codec::Format;
use gd::{Img, argb};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Extension, Registry, Throw};
use p8_runtime::object::{CLASS_FINAL, CLASS_NOT_SERIALIZABLE, CLASS_UNCLONEABLE, Native, Object};
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

pub const EXTENSION: Extension = Extension { name: "gd", register };

type R = Result<Value, Throw>;

/// A GdImage's native state.
#[derive(Default)]
struct Handle {
    im: RefCell<Option<Img>>,
}

const SUPPORTED: i64 = 1 | 2 | 4 | 8 | 32 | 64;

fn register(registry: &mut Registry) {
    registry.add_functions(BUILTINS);
    registry.add_functions(exif::BUILTINS);
    registry.add_classes(CLASSES);
    registry.add_data("GdImage", || Rc::new(Handle::default()));
    for (name, value) in CONSTANTS {
        registry.add_constant(name.as_bytes(), Constant::Int(*value));
    }
    registry.add_constant(b"GD_VERSION", Constant::Str(b"2.0.35"));
    registry.add_constant(b"GD_EXTRA_VERSION", Constant::Str(b""));
    // Output goes through the output buffers (whose callbacks run PHP code), and
    // other wrappers are read and written with PHP's own stream functions.
    registry.mark_reenters(&[
        "imagejpeg", "imagepng", "imagegif", "imagewebp", "imagebmp", "imagecreatefromjpeg", "imagecreatefrompng", "imagecreatefromgif",
        "imagecreatefromwebp", "imagecreatefrombmp", "imagecreatefromwbmp", "imagewbmp", "exif_read_data", "exif_imagetype", "exif_thumbnail",
    ]);
    registry.add_module("gd");
    registry.add_module("exif");
    registry.add_constant(b"EXIF_USE_MBSTRING", Constant::Int(1));
    for (name, default) in [
        ("exif.encode_unicode", &b"ISO-8859-15"[..]),
        ("exif.decode_unicode_motorola", b"UCS-2BE"),
        ("exif.decode_unicode_intel", b"UCS-2LE"),
        ("exif.encode_jis", b""),
        ("exif.decode_jis_motorola", b"JIS"),
        ("exif.decode_jis_intel", b"JIS"),
    ] {
        registry.add_ini(name, Some(default));
    }
}

static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "GdImage",
    parent: None,
    interfaces: &[],
    flags: CLASS_FINAL | CLASS_NOT_SERIALIZABLE | CLASS_UNCLONEABLE,
    properties: &[],
    methods: &[("__construct", Some(Native::Method(construct)))],
    constants: &[],
    debug_info: None,
}];

fn construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> R {
    Err(ctx.throw("Error", "You cannot initialize a GdImage object except through helper functions".into()))
}

static CONSTANTS: &[(&str, i64)] = &[
    ("IMG_AVIF", 256), ("IMG_GIF", 1), ("IMG_JPG", 2), ("IMG_JPEG", 2), ("IMG_PNG", 4), ("IMG_WBMP", 8), ("IMG_XPM", 16),
    ("IMG_WEBP", 32), ("IMG_BMP", 64), ("IMG_TGA", 128), ("IMG_WEBP_LOSSLESS", 101), ("IMG_COLOR_TILED", -5),
    ("IMG_COLOR_STYLED", -2), ("IMG_COLOR_BRUSHED", -3), ("IMG_COLOR_STYLEDBRUSHED", -4), ("IMG_COLOR_TRANSPARENT", -6),
    ("IMG_ARC_ROUNDED", 0), ("IMG_ARC_PIE", 0), ("IMG_ARC_CHORD", 1), ("IMG_ARC_NOFILL", 2), ("IMG_ARC_EDGED", 4),
    ("IMG_GD2_RAW", 1), ("IMG_GD2_COMPRESSED", 2), ("IMG_FLIP_HORIZONTAL", 1), ("IMG_FLIP_VERTICAL", 2), ("IMG_FLIP_BOTH", 3),
    ("IMG_EFFECT_REPLACE", 0), ("IMG_EFFECT_ALPHABLEND", 1), ("IMG_EFFECT_NORMAL", 2), ("IMG_EFFECT_OVERLAY", 3),
    ("IMG_EFFECT_MULTIPLY", 4), ("IMG_CROP_DEFAULT", 0), ("IMG_CROP_TRANSPARENT", 1), ("IMG_CROP_BLACK", 2),
    ("IMG_CROP_WHITE", 3), ("IMG_CROP_SIDES", 4), ("IMG_CROP_THRESHOLD", 5), ("IMG_BELL", 1), ("IMG_BESSEL", 2),
    ("IMG_BILINEAR_FIXED", 3), ("IMG_BICUBIC", 4), ("IMG_BICUBIC_FIXED", 5), ("IMG_BLACKMAN", 6), ("IMG_BOX", 7),
    ("IMG_BSPLINE", 8), ("IMG_CATMULLROM", 9), ("IMG_GAUSSIAN", 10), ("IMG_GENERALIZED_CUBIC", 11), ("IMG_HERMITE", 12),
    ("IMG_HAMMING", 13), ("IMG_HANNING", 14), ("IMG_MITCHELL", 15), ("IMG_POWER", 17), ("IMG_QUADRATIC", 18),
    ("IMG_SINC", 19), ("IMG_NEAREST_NEIGHBOUR", 16), ("IMG_WEIGHTED4", 21), ("IMG_TRIANGLE", 20),
    ("IMG_AFFINE_TRANSLATE", 0), ("IMG_AFFINE_SCALE", 1), ("IMG_AFFINE_ROTATE", 2), ("IMG_AFFINE_SHEAR_HORIZONTAL", 3),
    ("IMG_AFFINE_SHEAR_VERTICAL", 4), ("GD_BUNDLED", 1), ("IMG_FILTER_NEGATE", 0), ("IMG_FILTER_GRAYSCALE", 1),
    ("IMG_FILTER_BRIGHTNESS", 2), ("IMG_FILTER_CONTRAST", 3), ("IMG_FILTER_COLORIZE", 4), ("IMG_FILTER_EDGEDETECT", 5),
    ("IMG_FILTER_GAUSSIAN_BLUR", 7), ("IMG_FILTER_SELECTIVE_BLUR", 8), ("IMG_FILTER_EMBOSS", 6),
    ("IMG_FILTER_MEAN_REMOVAL", 9), ("IMG_FILTER_SMOOTH", 10), ("IMG_FILTER_PIXELATE", 11), ("IMG_FILTER_SCATTER", 12),
    ("GD_MAJOR_VERSION", 2), ("GD_MINOR_VERSION", 0), ("GD_RELEASE_VERSION", 35), ("PNG_NO_FILTER", 0),
    ("PNG_FILTER_NONE", 8), ("PNG_FILTER_SUB", 16), ("PNG_FILTER_UP", 32), ("PNG_FILTER_AVG", 64), ("PNG_FILTER_PAETH", 128),
    ("PNG_ALL_FILTERS", 248),
];

macro_rules! b {
    ($name:literal, $f:ident) => {
        Builtin { name: $name, run: $f, by_ref: &[] }
    };
}

static BUILTINS: &[Builtin] = &[
    b!("gd_info", gd_info),
    b!("imagetypes", imagetypes),
    b!("imagecreate", imagecreate),
    b!("imagecreatetruecolor", imagecreatetruecolor),
    b!("imagecreatefromstring", imagecreatefromstring),
    b!("imagecreatefromjpeg", imagecreatefromjpeg),
    b!("imagecreatefrompng", imagecreatefrompng),
    b!("imagecreatefromgif", imagecreatefromgif),
    b!("imagecreatefromwebp", imagecreatefromwebp),
    b!("imagecreatefrombmp", imagecreatefrombmp),
    b!("imagecreatefromwbmp", imagecreatefromwbmp),
    b!("imagewbmp", imagewbmp),
    b!("imagecolormatch", imagecolormatch),
    b!("imagedestroy", imagedestroy),
    b!("imagesx", imagesx),
    b!("imagesy", imagesy),
    b!("imageistruecolor", imageistruecolor),
    b!("imagepalettetotruecolor", imagepalettetotruecolor),
    b!("imagetruecolortopalette", imagetruecolortopalette),
    b!("imagepalettecopy", imagepalettecopy),
    b!("imagecolorstotal", imagecolorstotal),
    b!("imagealphablending", imagealphablending),
    b!("imagesavealpha", imagesavealpha),
    b!("imageantialias", imageantialias),
    b!("imagelayereffect", imagelayereffect),
    b!("imageinterlace", imageinterlace),
    b!("imagesetinterpolation", imagesetinterpolation),
    b!("imagegetinterpolation", imagegetinterpolation),
    b!("imageresolution", imageresolution),
    b!("imagesetthickness", imagesetthickness),
    b!("imagesetclip", imagesetclip),
    b!("imagegetclip", imagegetclip),
    b!("imagecolorallocate", imagecolorallocate),
    b!("imagecolorallocatealpha", imagecolorallocatealpha),
    b!("imagecolorat", imagecolorat),
    b!("imagecolorsforindex", imagecolorsforindex),
    b!("imagecolorclosest", imagecolorclosest),
    b!("imagecolorclosestalpha", imagecolorclosestalpha),
    b!("imagecolorexact", imagecolorexact),
    b!("imagecolorexactalpha", imagecolorexactalpha),
    b!("imagecolorresolve", imagecolorresolve),
    b!("imagecolorresolvealpha", imagecolorresolvealpha),
    b!("imagecolordeallocate", imagecolordeallocate),
    b!("imagecolorset", imagecolorset),
    b!("imagecolortransparent", imagecolortransparent),
    b!("imagesetpixel", imagesetpixel),
    b!("imageline", imageline),
    b!("imagerectangle", imagerectangle),
    b!("imagefilledrectangle", imagefilledrectangle),
    b!("imageellipse", imageellipse),
    b!("imagefilledellipse", imagefilledellipse),
    b!("imagearc", imagearc),
    b!("imagefilledarc", imagefilledarc),
    b!("imagepolygon", imagepolygon),
    b!("imageopenpolygon", imageopenpolygon),
    b!("imagefilledpolygon", imagefilledpolygon),
    b!("imagefill", imagefill),
    b!("imagefilltoborder", imagefilltoborder),
    b!("imagecopy", imagecopy),
    b!("imagecopymerge", imagecopymerge),
    b!("imagecopymergegray", imagecopymergegray),
    b!("imagecopyresized", imagecopyresized),
    b!("imagecopyresampled", imagecopyresampled),
    b!("imagecrop", imagecrop),
    b!("imagecropauto", imagecropauto),
    b!("imagescale", imagescale),
    b!("imagerotate", imagerotate),
    b!("imageflip", imageflip),
    b!("imagefilter", imagefilter),
    b!("imageconvolution", imageconvolution),
    b!("imagegammacorrect", imagegammacorrect),
    b!("imagejpeg", imagejpeg),
    b!("imagepng", imagepng),
    b!("imagegif", imagegif),
    b!("imagewebp", imagewebp),
    b!("imagebmp", imagebmp),
];

// ---- argument helpers ----

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(Value::deref).unwrap_or(Value::NULL)
}

fn int(args: &[Value], i: usize) -> i64 {
    arg(args, i).to_int()
}

/// An int argument as gd's C int.
fn ci(args: &[Value], i: usize) -> i32 {
    int(args, i).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn given(args: &[Value], i: usize) -> bool {
    !arg(args, i).is_null()
}

fn object(args: &[Value], i: usize) -> Rc<Object> {
    match arg(args, i).into_object() {
        Ok(o) => o,
        Err(_) => unreachable!("checked against the stub"),
    }
}

/// Run `f` on the GdImage in argument `i`.
fn with<T>(args: &[Value], i: usize, f: impl FnOnce(&mut Img) -> T) -> T {
    let o = object(args, i);
    let h = o.native::<Handle>().expect("GdImage state");
    let mut cell = h.im.borrow_mut();
    f(cell.as_mut().expect("GdImage image"))
}

fn read<T>(args: &[Value], i: usize, f: impl FnOnce(&Img) -> T) -> T {
    with(args, i, |im| f(im))
}

fn new_image(ctx: &mut dyn Ctx, im: Img) -> Value {
    let class = ctx.class(b"GdImage").expect("GdImage");
    let o = ctx.new_object(&class);
    Value::object(o.sibling(class, Some(Rc::new(Handle { im: RefCell::new(Some(im)) }))))
}

fn value_error(ctx: &mut dyn Ctx, arg: usize, name: &str, msg: &str) -> Throw {
    let f = ctx.builtin_name();
    ctx.throw("ValueError", format!("{f}(): Argument #{arg} (${name}) {msg}"))
}

fn ok() -> R {
    Ok(Value::TRUE)
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

// ---- info ----

fn gd_info(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    let mut a = PhpArray::new();
    let mut set = |k: &str, v: Value| a.insert(Key::from_string(k.into()), v);
    set("GD Version", Value::str("bundled (2.1.0 compatible)"));
    set("FreeType Support", Value::FALSE);
    set("GIF Read Support", Value::TRUE);
    set("GIF Create Support", Value::TRUE);
    set("JPEG Support", Value::TRUE);
    set("PNG Support", Value::TRUE);
    set("WBMP Support", Value::TRUE);
    set("XPM Support", Value::FALSE);
    set("XBM Support", Value::FALSE);
    set("WebP Support", Value::TRUE);
    set("BMP Support", Value::TRUE);
    set("AVIF Support", Value::FALSE);
    set("TGA Read Support", Value::FALSE);
    set("JIS-mapped Japanese Font Support", Value::FALSE);
    Ok(Value::array(a))
}

fn imagetypes(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    Ok(Value::int(SUPPORTED))
}

// ---- creating ----

fn create(ctx: &mut dyn Ctx, args: &[Value], tc: bool) -> R {
    let (w, h) = (int(args, 0), int(args, 1));
    if w <= 0 || w >= i64::from(i32::MAX) {
        return Err(value_error(ctx, 1, "width", "must be greater than 0"));
    }
    if h <= 0 || h >= i64::from(i32::MAX) {
        return Err(value_error(ctx, 2, "height", "must be greater than 0"));
    }
    if w.checked_mul(h).is_none_or(|n| n > (1 << 28)) {
        let f = ctx.builtin_name();
        ctx.warn(&format!("{f}(): Product of memory allocation multiplication would exceed INT_MAX, failing operation gracefully\n"));
        return Ok(Value::FALSE);
    }
    let mut im = Img::new(w as i32, h as i32, tc);
    if !tc {
        im.blend = false;
    }
    Ok(new_image(ctx, im))
}

fn imagecreate(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    create(ctx, args, false)
}

fn imagecreatetruecolor(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    create(ctx, args, true)
}

fn imagecreatefromstring(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let data = arg(args, 0).to_php_string().unwrap_or_else(p8_runtime::string::PhpStr::empty);
    let sniffed = if data.as_bytes().len() < 12 { None } else { codec::sniff(data.as_bytes()) };
    let Some(format) = sniffed else {
        ctx.warn("imagecreatefromstring(): Data is not in a recognized format");
        return Ok(Value::FALSE);
    };
    match codec::decode(format, data.as_bytes()) {
        Some(im) => Ok(new_image(ctx, im)),
        None => {
            if format == Format::Webp {
                ctx.warn("imagecreatefromstring(): gd-webp cannot get webp info");
            }
            ctx.warn(&format!("imagecreatefromstring(): Passed data is not in \"{}\" format", format.name()));
            ctx.warn("imagecreatefromstring(): Couldn't create GD Image Stream out of Data");
            Ok(Value::FALSE)
        }
    }
}

/// The bytes a filename names: a plain path is read directly; any other
/// wrapper through file_get_contents().
fn read_input(ctx: &mut dyn Ctx, func: &str, path: &[u8]) -> Result<Option<Vec<u8>>, Throw> {
    if path.contains(&0) {
        return Err(value_error(ctx, 1, "filename", "must not contain any null bytes"));
    }
    let shown = lossy(path);
    if shown.contains("://") && !shown.starts_with("file://") || shown.starts_with("data:") {
        let r = ctx.call(&Value::str("file_get_contents"), &mut [Value::str(shown.as_str())])?;
        return Ok(r.as_str().map(|s| s.as_bytes().to_vec()));
    }
    let p = shown.strip_prefix("file://").unwrap_or(&shown);
    match std::fs::read(p) {
        Ok(d) => Ok(Some(d)),
        Err(e) => {
            ctx.warn(&format!("{func}({shown}): Failed to open stream: {}", os_error(&e)));
            Ok(None)
        }
    }
}

fn os_error(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory".into(),
        std::io::ErrorKind::PermissionDenied => "Permission denied".into(),
        std::io::ErrorKind::IsADirectory => "Is a directory".into(),
        _ => {
            let s = e.to_string();
            match s.find(" (os error") {
                Some(at) => s[..at].to_string(),
                None => s,
            }
        }
    }
}

fn from_file(ctx: &mut dyn Ctx, args: &[Value], func: &'static str, format: Format) -> R {
    let path = arg(args, 0).to_php_string().unwrap_or_else(p8_runtime::string::PhpStr::empty);
    let Some(data) = read_input(ctx, func, path.as_bytes())? else { return Ok(Value::FALSE) };
    match codec::decode(format, &data) {
        Some(im) => Ok(new_image(ctx, im)),
        None => {
            if format == Format::Jpeg && !data.starts_with(&[0xff, 0xd8]) {
                let b = |i: usize| data.get(i).copied().unwrap_or(0);
                ctx.warn(&format!("{func}(): gd-jpeg: JPEG library reports unrecoverable error: Not a JPEG file: starts with 0x{:02x} 0x{:02x}", b(0), b(1)));
            }
            if format == Format::Webp {
                ctx.warn(&format!("{func}(): gd-webp cannot get webp info"));
            }
            ctx.warn(&format!("{func}(): \"{}\" is not a valid {} file", lossy(path.as_bytes()), format.name()));
            Ok(Value::FALSE)
        }
    }
}

fn imagecreatefromjpeg(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    from_file(ctx, args, "imagecreatefromjpeg", Format::Jpeg)
}
fn imagecreatefrompng(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    from_file(ctx, args, "imagecreatefrompng", Format::Png)
}
fn imagecreatefromgif(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    from_file(ctx, args, "imagecreatefromgif", Format::Gif)
}
fn imagecreatefromwebp(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    from_file(ctx, args, "imagecreatefromwebp", Format::Webp)
}
fn imagecreatefrombmp(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    from_file(ctx, args, "imagecreatefrombmp", Format::Bmp)
}

fn imagecreatefromwbmp(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    from_file(ctx, args, "imagecreatefromwbmp", Format::Wbmp)
}

fn imagewbmp(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    check_target(ctx, "imagewbmp", &arg(args, 1))?;
    let fg = if given(args, 2) { ci(args, 2) } else { 0 };
    let bytes = read(args, 0, |im| codec::encode_wbmp(im, fg));
    send(ctx, "imagewbmp", &arg(args, 1), &bytes)
}

fn imagecolormatch(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (a, b) = (object(args, 0), object(args, 1));
    if Rc::ptr_eq(&a, &b) {
        let tc = read(args, 0, |im| im.tc);
        return Err(if tc { value_error(ctx, 2, "image2", "must be Palette") } else { value_error(ctx, 1, "image1", "must be TrueColor") });
    }
    let first = read(args, 0, Img::clone);
    match with(args, 1, |second| gd::color_match(&first, second)) {
        Ok(()) => ok(),
        Err(-1) => Err(value_error(ctx, 1, "image1", "must be TrueColor")),
        Err(-2) => Err(value_error(ctx, 2, "image2", "must be Palette")),
        Err(-3) => Err(value_error(ctx, 2, "image2", "must be the same size as argument #1 ($im1)")),
        Err(_) => Err(value_error(ctx, 2, "image2", "must have at least one color")),
    }
}

fn imagedestroy(_: &mut dyn Ctx, _: &mut [Value]) -> R {
    ok()
}

// ---- properties ----

fn imagesx(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::int(i64::from(read(args, 0, |im| im.w))))
}

fn imagesy(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::int(i64::from(read(args, 0, |im| im.h))))
}

fn imageistruecolor(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::bool(read(args, 0, |im| im.tc)))
}

fn imagepalettetotruecolor(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    with(args, 0, |im| im.make_truecolor());
    ok()
}

fn imagetruecolortopalette(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let n = int(args, 2);
    if n <= 0 || n >= i64::from(i32::MAX) {
        return Err(value_error(ctx, 3, "num_colors", "must be greater than 0 and less than 2147483647"));
    }
    with(args, 0, |im| {
        if !im.tc {
            return;
        }
        let (pal, idx, trans) = gd::quantize(im, (n as usize).min(gd::MAX_COLORS));
        im.pal = pal;
        im.px = idx.iter().map(|&i| i32::from(i)).collect();
        im.transparent = trans.map_or(-1, i32::from);
        im.tc = false;
        im.blend = false;
    });
    ok()
}

fn imagepalettecopy(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let pal = read(args, 1, |im| im.pal.clone());
    with(args, 0, |im| {
        if !im.tc {
            im.pal = pal;
        }
    });
    ok()
}

fn imagecolorstotal(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::int(read(args, 0, |im| im.pal.len() as i64)))
}

fn imagealphablending(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let on = arg(args, 1).to_bool();
    with(args, 0, |im| im.blend = on);
    ok()
}

fn imagesavealpha(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let on = arg(args, 1).to_bool();
    with(args, 0, |im| im.save_alpha = on);
    ok()
}

fn imageantialias(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::bool(read(args, 0, |im| im.tc)))
}

fn imagelayereffect(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let effect = int(args, 1);
    with(args, 0, |im| im.blend = effect != 0);
    ok()
}

fn imageinterlace(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let set = given(args, 1).then(|| arg(args, 1).to_bool());
    Ok(Value::bool(with(args, 0, |im| {
        if let Some(on) = set {
            im.interlace = on;
        }
        im.interlace
    })))
}

fn imagesetinterpolation(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let mut method = if given(args, 1) { int(args, 1) } else { 3 };
    if method == -1 {
        method = 3;
    }
    if !(0..=21).contains(&method) {
        return Ok(Value::FALSE);
    }
    if method == 0 {
        method = 3;
    }
    with(args, 0, |im| im.interp = method);
    ok()
}

fn imagegetinterpolation(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    Ok(Value::int(read(args, 0, |im| im.interp)))
}

fn imageresolution(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    if !given(args, 1) && !given(args, 2) {
        let (x, y) = read(args, 0, |im| im.res);
        return Ok(Value::array(PhpArray::from_list(vec![Value::int(x), Value::int(y)])));
    }
    let range = |ctx: &mut dyn Ctx, v: i64, n: usize, name: &str| -> Result<(), Throw> {
        if (0..=i64::from(u32::MAX)).contains(&v) { Ok(()) } else { Err(value_error(ctx, n, name, "must be between 0 and 4294967295")) }
    };
    let x = given(args, 1).then(|| int(args, 1));
    let y = given(args, 2).then(|| int(args, 2));
    if let Some(v) = x {
        range(ctx, v, 2, "resolution_x")?;
    }
    if let Some(v) = y {
        range(ctx, v, 3, "resolution_y")?;
    }
    let (x, y) = (x.or(y).unwrap_or(0), y.or(x).unwrap_or(0));
    with(args, 0, |im| {
        if x > 0 {
            im.res.0 = x;
        }
        if y > 0 {
            im.res.1 = y;
        }
    });
    ok()
}

fn imagesetthickness(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let t = ci(args, 1);
    with(args, 0, |im| im.thick = t);
    ok()
}

fn imagesetclip(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (x1, y1, x2, y2) = (ci(args, 1), ci(args, 2), ci(args, 3), ci(args, 4));
    with(args, 0, |im| {
        let c = |v: i32, hi: i32| v.clamp(0, hi);
        let (x1, x2) = (c(x1, im.w - 1), c(x2, im.w - 1));
        let (y1, y2) = (c(y1, im.h - 1), c(y2, im.h - 1));
        im.clip = (x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2));
    });
    ok()
}

fn imagegetclip(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (a, b, c, d) = read(args, 0, |im| im.clip);
    Ok(Value::array(PhpArray::from_list(vec![Value::int(a.into()), Value::int(b.into()), Value::int(c.into()), Value::int(d.into())])))
}

// ---- colours ----

const COLOR_NAMES: [&str; 3] = ["red", "green", "blue"];

/// The red, green, blue (and alpha, argument #5) of a colour call, range-checked.
fn rgba(ctx: &mut dyn Ctx, args: &[Value], with_alpha: bool) -> Result<(i32, i32, i32, i32), Throw> {
    let mut v = [0; 4];
    for i in 0..3 {
        let c = int(args, i + 1);
        if !(0..=255).contains(&c) {
            return Err(value_error(ctx, i + 2, COLOR_NAMES[i], "must be between 0 and 255 (inclusive)"));
        }
        v[i] = c as i32;
    }
    if with_alpha {
        let a = int(args, 4);
        if !(0..=127).contains(&a) {
            return Err(value_error(ctx, 5, "alpha", "must be between 0 and 127 (inclusive)"));
        }
        v[3] = a as i32;
    }
    Ok((v[0], v[1], v[2], v[3]))
}

fn alloc(ctx: &mut dyn Ctx, args: &[Value], with_alpha: bool) -> R {
    let (r, g, b, a) = rgba(ctx, args, with_alpha)?;
    let c = with(args, 0, |im| im.allocate(r, g, b, a));
    Ok(if c == -1 { Value::FALSE } else { Value::int(i64::from(c)) })
}

fn imagecolorallocate(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    alloc(ctx, args, false)
}

fn imagecolorallocatealpha(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    alloc(ctx, args, true)
}

fn imagecolorat(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (x, y) = (ci(args, 1), ci(args, 2));
    let v = read(args, 0, |im| if im.safe(x, y) { Some(im.get(x, y)) } else { None });
    match v {
        Some(c) => Ok(Value::int(i64::from(c))),
        None => {
            ctx.warn(&format!("imagecolorat(): {x},{y} is out of bounds"));
            Ok(Value::FALSE)
        }
    }
}

fn imagecolorsforindex(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let c = int(args, 1);
    let r = read(args, 0, |im| {
        if im.tc {
            let c = c as i32;
            Some((gd::red(c), gd::green(c), gd::blue(c), gd::alpha(c)))
        } else if c >= 0 && (c as usize) < im.pal.len() {
            let p = im.pal[c as usize];
            Some((p.r, p.g, p.b, p.a))
        } else {
            None
        }
    });
    let Some((r, g, b, a)) = r else { return Err(value_error(ctx, 2, "color", "is out of range")) };
    let mut out = PhpArray::new();
    for (k, v) in [("red", r), ("green", g), ("blue", b), ("alpha", a)] {
        out.insert(Key::from_string(k.into()), Value::int(i64::from(v)));
    }
    Ok(Value::array(out))
}

fn find(ctx: &mut dyn Ctx, args: &[Value], with_alpha: bool, how: fn(&mut Img, i32, i32, i32, i32) -> i32) -> R {
    let (r, g, b, a) = rgba(ctx, args, with_alpha)?;
    Ok(Value::int(i64::from(with(args, 0, |im| how(im, r, g, b, a)))))
}

fn imagecolorclosest(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    find(ctx, args, false, |im, r, g, b, a| im.closest(r, g, b, a))
}
fn imagecolorclosestalpha(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    find(ctx, args, true, |im, r, g, b, a| im.closest(r, g, b, a))
}
fn imagecolorexact(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    find(ctx, args, false, |im, r, g, b, a| im.exact(r, g, b, a))
}
fn imagecolorexactalpha(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    find(ctx, args, true, |im, r, g, b, a| im.exact(r, g, b, a))
}
fn imagecolorresolve(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    find(ctx, args, false, |im, r, g, b, a| im.resolve(r, g, b, a))
}
fn imagecolorresolvealpha(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    find(ctx, args, true, |im, r, g, b, a| im.resolve(r, g, b, a))
}

fn imagecolordeallocate(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let c = int(args, 1);
    let (tc, total) = read(args, 0, |im| (im.tc, im.pal.len() as i64));
    if tc {
        return ok();
    }
    if c < 0 || c >= total {
        return Err(value_error(ctx, 2, "color", &format!("must be between 0 and {total}")));
    }
    with(args, 0, |im| im.pal[c as usize].open = true);
    ok()
}

fn imagecolorset(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let c = int(args, 1);
    for (i, name) in [(2, "red"), (3, "green"), (4, "blue")] {
        if !(0..=255).contains(&int(args, i)) {
            return Err(value_error(ctx, i + 1, name, "must be between 0 and 255 (inclusive)"));
        }
    }
    if given(args, 5) && !(0..=127).contains(&int(args, 5)) {
        return Err(value_error(ctx, 6, "alpha", "must be between 0 and 127 (inclusive)"));
    }
    let ch = |i: usize| int(args, i) as i32;
    let (r, g, b, a) = (ch(2), ch(3), ch(4), int(args, 5) as i32);
    let set = with(args, 0, |im| {
        if !im.tc && c >= 0 && (c as usize) < im.pal.len() {
            im.pal[c as usize] = gd::Pal { r, g, b, a, open: false };
            true
        } else {
            false
        }
    });
    Ok(if set { Value::NULL } else { Value::FALSE })
}

fn imagecolortransparent(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let set = given(args, 1).then(|| ci(args, 1));
    Ok(Value::int(i64::from(with(args, 0, |im| {
        if let Some(c) = set
            && (im.tc || (c >= -1 && (c as isize) < im.pal.len() as isize))
        {
            im.transparent = c;
        }
        im.transparent
    }))))
}

// ---- drawing ----

fn imagesetpixel(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (x, y, c) = (ci(args, 1), ci(args, 2), ci(args, 3));
    with(args, 0, |im| im.set_pixel(x, y, c));
    ok()
}

fn imageline(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (a, b, c, d, col) = (ci(args, 1), ci(args, 2), ci(args, 3), ci(args, 4), ci(args, 5));
    with(args, 0, |im| gd::line(im, a, b, c, d, col));
    ok()
}

fn imagerectangle(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (a, b, c, d, col) = (ci(args, 1), ci(args, 2), ci(args, 3), ci(args, 4), ci(args, 5));
    with(args, 0, |im| gd::rectangle(im, a, b, c, d, col));
    ok()
}

fn imagefilledrectangle(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (a, b, c, d, col) = (ci(args, 1), ci(args, 2), ci(args, 3), ci(args, 4), ci(args, 5));
    with(args, 0, |im| gd::filled_rectangle(im, a, b, c, d, col));
    ok()
}

fn imageellipse(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (cx, cy, w, h, col) = (ci(args, 1), ci(args, 2), ci(args, 3), ci(args, 4), ci(args, 5));
    with(args, 0, |im| gd::ellipse(im, cx, cy, w, h, col, false));
    ok()
}

fn imagefilledellipse(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (cx, cy, w, h, col) = (ci(args, 1), ci(args, 2), ci(args, 3), ci(args, 4), ci(args, 5));
    with(args, 0, |im| gd::ellipse(im, cx, cy, w, h, col, true));
    ok()
}

/// imagepolygon(image, points, num_points_or_color, color = null).
fn poly(ctx: &mut dyn Ctx, args: &[Value]) -> Result<(Vec<(i32, i32)>, i32), Throw> {
    let Ok(a) = arg(args, 1).into_array() else { unreachable!("checked against the stub") };
    let vals: Vec<i32> = a.values().map(|v| v.deref().to_int().clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32).collect();
    let (num, color) = if given(args, 3) {
        let f = ctx.builtin_name();
        ctx.deprecated(&format!("{f}(): Using the $num_points parameter is deprecated"));
        (int(args, 2), ci(args, 3))
    } else {
        ((vals.len() / 2) as i64, ci(args, 2))
    };
    if num < 3 {
        return Err(value_error(ctx, 3, "num_points_or_color", "must be greater than or equal to 3"));
    }
    if (vals.len() as i64) < num * 2 {
        return Err(value_error(ctx, 2, "points", "must have at least 2 * argument #3 ($num_points_or_color) elements"));
    }
    Ok(((0..num as usize).map(|i| (vals[2 * i], vals[2 * i + 1])).collect(), color))
}

fn imagearc(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let a: Vec<i32> = (1..=7).map(|i| ci(args, i)).collect();
    with(args, 0, |im| gd::arc(im, a[0], a[1], a[2], a[3], a[4], a[5], a[6], gd::ARC_NOFILL));
    ok()
}

fn imagefilledarc(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let a: Vec<i32> = (1..=8).map(|i| ci(args, i)).collect();
    with(args, 0, |im| gd::arc(im, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]));
    ok()
}

fn imagepolygon(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (pts, col) = poly(ctx, args)?;
    with(args, 0, |im| gd::polygon(im, &pts, col, true));
    ok()
}

fn imageopenpolygon(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (pts, col) = poly(ctx, args)?;
    with(args, 0, |im| gd::polygon(im, &pts, col, false));
    ok()
}

fn imagefilledpolygon(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (pts, col) = poly(ctx, args)?;
    with(args, 0, |im| gd::filled_polygon(im, &pts, col));
    ok()
}

fn imagefill(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (x, y, c) = (ci(args, 1), ci(args, 2), ci(args, 3));
    with(args, 0, |im| gd::fill(im, x, y, c));
    ok()
}

fn imagefilltoborder(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (x, y, border, c) = (ci(args, 1), ci(args, 2), ci(args, 3), ci(args, 4));
    with(args, 0, |im| gd::fill_to_border(im, x, y, border, c));
    ok()
}

// ---- copying and transforming ----

/// Run `f` on the destination image (argument 0) and a copy of the source
/// (argument 1), which may be the same image.
fn pair(args: &[Value], f: impl FnOnce(&mut Img, &Img)) {
    let src = read(args, 1, Img::clone);
    with(args, 0, |dst| f(dst, &src));
}

fn imagecopy(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    let a: Vec<i32> = (2..8).map(|i| ci(args, i)).collect();
    pair(args, |d, s| gd::copy(d, s, a[0], a[1], a[2], a[3], a[4], a[5]));
    ok()
}

fn imagecopymerge(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    merge(args, false)
}

fn imagecopymergegray(_: &mut dyn Ctx, args: &mut [Value]) -> R {
    merge(args, true)
}

fn merge(args: &[Value], gray: bool) -> R {
    let a: Vec<i32> = (2..9).map(|i| ci(args, i)).collect();
    pair(args, |d, s| {
        if a[6] >= 100 && !gray {
            gd::copy(d, s, a[0], a[1], a[2], a[3], a[4], a[5]);
        } else if a[6] > 0 {
            gd::copy_merge(d, s, a[0], a[1], a[2], a[3], a[4], a[5], a[6], gray);
        }
    });
    ok()
}

fn imagecopyresized(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    resize(ctx, args, false)
}

fn imagecopyresampled(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    resize(ctx, args, true)
}

fn resize(ctx: &mut dyn Ctx, args: &[Value], smooth: bool) -> R {
    let a: Vec<i32> = (2..10).map(|i| ci(args, i)).collect();
    if !smooth {
        for (k, n, name) in [(4, 7, "dst_width"), (5, 8, "dst_height"), (6, 9, "src_width"), (7, 10, "src_height")] {
            if a[k] <= 0 {
                return Err(value_error(ctx, n, name, "must be greater than 0"));
            }
        }
    }
    pair(args, |d, s| {
        if smooth {
            gd::copy_resampled(d, s, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]);
        } else {
            gd::copy_resized(d, s, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]);
        }
    });
    ok()
}

fn rect_field(ctx: &mut dyn Ctx, rect: &PhpArray, key: &str, missing: &str) -> Result<i32, Throw> {
    match rect.get(&Key::from_string(key.into())) {
        Some(v) => Ok(v.deref().to_int().clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32),
        None => Err(ctx.throw("ValueError", missing.into())),
    }
}

fn imagecrop(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Ok(rect) = arg(args, 1).into_array() else { unreachable!("checked against the stub") };
    let x = rect_field(ctx, &rect, "x", "imagecrop(): Argument #2 ($rectangle) must have a \"x\" key")?;
    let y = rect_field(ctx, &rect, "y", "imagecrop(): Argument #2 ($rectangle) must have a \"y\" key")?;
    let w = rect_field(ctx, &rect, "width", "imagecrop(): Argument #2 ($rectangle) must have a \"width\" key")?;
    let h = rect_field(ctx, &rect, "height", "imagecrop(): Argument #2 ($rectangle) must have a \"height\" key")?;
    for (pos, len, p, l) in [(x, w, "x", "width"), (y, h, "y", "height")] {
        if pos > 0 && len > i32::MAX - pos {
            return Err(ctx.throw("ValueError", format!("imagecrop(): Argument #2 ($rectangle) overflow with \"{p}\" and \"{l}\" keys")));
        }
        if pos < 0 && len < i32::MIN - pos {
            return Err(ctx.throw("ValueError", format!("imagecrop(): Argument #2 ($rectangle) underflow with \"{p}\" and \"{l}\" keys")));
        }
    }
    if w <= 0 || h <= 0 {
        ctx.warn("imagecrop(): One parameter to a memory allocation multiplication is negative or zero, failing operation gracefully\n");
        return Ok(Value::FALSE);
    }
    if i64::from(w) * i64::from(h) > (1 << 28) {
        return Ok(Value::FALSE);
    }
    let out = read(args, 0, |im| gd::crop(im, x, y, w, h));
    Ok(new_image(ctx, out))
}

fn imagecropauto(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let mode = if given(args, 1) { int(args, 1) } else { 0 };
    let threshold = if given(args, 2) { arg(args, 2).to_float() } else { 0.5 };
    let color = if given(args, 3) { ci(args, 3) } else { -1 };
    if !(0..=5).contains(&mode) {
        return Err(value_error(ctx, 2, "mode", "must be a valid mode"));
    }
    if mode == 5 {
        if color < 0 {
            return Err(value_error(ctx, 4, "color", "must be greater than or equal to 0 when using the threshold mode"));
        }
        if !(0.0..=1.0).contains(&threshold) {
            return Err(value_error(ctx, 3, "threshold", "must be between 0.0 and 1.0"));
        }
    }
    let r = read(args, 0, |im| crop_auto(im, mode, threshold, color));
    match r {
        Some(out) => Ok(new_image(ctx, out)),
        None => Ok(Value::FALSE),
    }
}

/// gdImageCropAuto: crop away the border of one colour.
fn crop_auto(im: &Img, mode: i64, threshold: f64, color: i32) -> Option<Img> {
    let t = gd::truecolor_copy(im);
    let corner = |x, y| t.get_tc(x, y);
    let (w, h) = (t.w, t.h);
    let same = |a: i32, b: i32| a == b;
    let (matches, _): (Box<dyn Fn(i32) -> bool>, ()) = match mode {
        1 => (Box::new(|c| gd::alpha(c) == gd::ALPHA_MAX), ()),
        2 => (Box::new(|c| c & 0xffffff == 0 && gd::alpha(c) == 0), ()),
        3 => (Box::new(|c| c & 0xffffff == 0xffffff && gd::alpha(c) == 0), ()),
        5 => {
            let ref_c = if color >= 0 { if im.tc { color } else { im.pal_tc(color) } } else { corner(0, 0) };
            let (rr, rg, rb) = (gd::red(ref_c), gd::green(ref_c), gd::blue(ref_c));
            (
                Box::new(move |c| {
                    let d = f64::from((gd::red(c) - rr).pow(2) + (gd::green(c) - rg).pow(2) + (gd::blue(c) - rb).pow(2));
                    (d.sqrt() / 441.672_955_930_063_7) <= threshold / 100.0 + 0.0 || d == 0.0
                }),
                (),
            )
        }
        _ => {
            // DEFAULT and SIDES: the colour of the corners.
            let c0 = corner(0, 0);
            let corners = [corner(w - 1, 0), corner(0, h - 1), corner(w - 1, h - 1)];
            if mode == 0 {
                (Box::new(|c| gd::alpha(c) == gd::ALPHA_MAX), ())
            } else if corners.iter().filter(|&&c| same(c, c0)).count() >= 2 {
                (Box::new(move |c| c == c0), ())
            } else {
                return Some(gd::crop(im, 0, 0, w, h));
            }
        }
    };
    let row_blank = |y: i32| (0..w).all(|x| matches(t.get_tc(x, y)));
    let col_blank = |x: i32| (0..h).all(|y| matches(t.get_tc(x, y)));
    let top = (0..h).find(|&y| !row_blank(y))?;
    let bottom = (0..h).rev().find(|&y| !row_blank(y))?;
    let left = (0..w).find(|&x| !col_blank(x))?;
    let right = (0..w).rev().find(|&x| !col_blank(x))?;
    Some(gd::crop(im, left, top, right - left + 1, bottom - top + 1))
}

fn imagescale(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let mut w = int(args, 1);
    let mut h = if given(args, 2) { int(args, 2) } else { -1 };
    let mode = if given(args, 3) { int(args, 3) } else { 3 };
    if !(0..=21).contains(&mode) {
        return Err(value_error(ctx, 4, "mode", "must be one of the GD_* constants"));
    }
    if h < 0 && w < 0 {
        return Err(ctx.throw("ValueError", "Argument #2 ($width) and argument #3 ($height) cannot be both negative".into()));
    }
    let (sx, sy) = read(args, 0, |im| (i64::from(im.w), i64::from(im.h)));
    if h < 0 || w < 0 {
        if sx != 0 && h < 0 {
            if w > i64::MAX / sy {
                return Err(value_error(ctx, 2, "width", &format!("must be less than or equal to {}", i64::MAX / sy)));
            }
            h = w * sy / sx;
        }
        if sy != 0 && w < 0 {
            if h > i64::MAX / sx {
                return Err(value_error(ctx, 3, "height", &format!("must be less than or equal to {}", i64::MAX / sx)));
            }
            w = h * sx / sy;
        }
    }
    let max = i64::from(i32::MAX);
    if w <= 0 || w > max {
        return Err(value_error(ctx, 2, "width", "must be between 1 and 2147483647"));
    }
    if h <= 0 || h > max {
        return Err(value_error(ctx, 3, "height", "must be between 1 and 2147483647"));
    }
    if w * h > (1 << 28) {
        return Ok(Value::FALSE);
    }
    let out = read(args, 0, |im| gd::scale(im, w as i32, h as i32, mode == 16, mode == 3 || mode == 5 || mode == 0));
    Ok(new_image(ctx, out))
}

fn imagerotate(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let angle = arg(args, 1).to_float();
    let (lo, hi) = (i32::MIN / 100, i32::MAX / 100);
    if angle < f64::from(lo) || angle > f64::from(hi) {
        return Err(value_error(ctx, 2, "angle", &format!("must be between {lo} and {hi}")));
    }
    let bg = ci(args, 2);
    if bg < 0 {
        return Ok(Value::FALSE);
    }
    let out = read(args, 0, |im| gd::rotate(im, angle, bg));
    Ok(new_image(ctx, out))
}

fn imageflip(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let mode = int(args, 1);
    if !(1..=3).contains(&mode) {
        return Err(value_error(ctx, 2, "mode", "must be one of IMG_FLIP_VERTICAL, IMG_FLIP_HORIZONTAL, or IMG_FLIP_BOTH"));
    }
    with(args, 0, |im| gd::flip(im, mode & 1 != 0, mode & 2 != 0));
    ok()
}

fn imagegammacorrect(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let (input, output) = (arg(args, 1).to_float(), arg(args, 2).to_float());
    for (v, n, name) in [(input, 2, "input_gamma"), (output, 3, "output_gamma")] {
        if v <= 0.0 {
            return Err(value_error(ctx, n, name, "must be greater than 0"));
        }
        if !v.is_finite() {
            return Err(value_error(ctx, n, name, "must be finite"));
        }
    }
    with(args, 0, |im| {
        let map = |v: i32| ((f64::from(v) / 255.0).powf(input).powf(1.0 / output) * 255.0 + 0.5) as i32;
        if im.tc {
            for p in im.px.iter_mut() {
                *p = argb(gd::alpha(*p), map(gd::red(*p)), map(gd::green(*p)), map(gd::blue(*p)));
            }
        } else {
            for p in im.pal.iter_mut() {
                (p.r, p.g, p.b) = (map(p.r), map(p.g), map(p.b));
            }
        }
    });
    ok()
}

/// A truecolor image's pixels through `f` (palette images are converted
/// first, as the filters work on truecolor).
fn per_pixel(im: &mut Img, f: impl Fn(i32, i32, i32, i32) -> (i32, i32, i32)) {
    let each = |c: i32| {
        let (r, g, b) = f(gd::red(c), gd::green(c), gd::blue(c), gd::alpha(c));
        argb(gd::alpha(c), r.clamp(0, 255), g.clamp(0, 255), b.clamp(0, 255))
    };
    if im.tc {
        im.px.iter_mut().for_each(|p| *p = each(*p));
    } else {
        for p in im.pal.iter_mut() {
            let (r, g, b) = f(p.r, p.g, p.b, p.a);
            (p.r, p.g, p.b) = (r.clamp(0, 255), g.clamp(0, 255), b.clamp(0, 255));
        }
    }
}

/// A pixel with coordinates clamped into the image (the filters read their
/// neighbourhood this way).
fn edge_pixel(im: &Img, x: i32, y: i32) -> i32 {
    im.get_tc(x.clamp(0, im.w - 1), y.clamp(0, im.h - 1))
}

fn convolve(im: &mut Img, m: [[f32; 3]; 3], div: f32, offset: f32) {
    let src = im.clone();
    for y in 0..im.h {
        for x in 0..im.w {
            let (mut r, mut g, mut b) = (0.0f32, 0.0f32, 0.0f32);
            for (j, row) in m.iter().enumerate() {
                for (i, k) in row.iter().enumerate() {
                    let p = edge_pixel(&src, x - 1 + i as i32, y - 1 + j as i32);
                    r += gd::red(p) as f32 * k;
                    g += gd::green(p) as f32 * k;
                    b += gd::blue(p) as f32 * k;
                }
            }
            let f = |v: f32| (v / div + offset).clamp(0.0, 255.0) as i32;
            let a = gd::alpha(src.get_tc(x, y));
            let i = (y as usize) * (im.w as usize) + x as usize;
            im.px[i] = argb(a, f(r), f(g), f(b));
        }
    }
}

/// gdImageSelectiveBlur: neighbours weigh in by how close they are in colour.
#[allow(clippy::needless_range_loop)] // the 3x3 window indexes the weights and the pixels together
fn selective_blur(im: &mut Img) {
    let src = im.clone();
    for y in 0..im.h {
        for x in 0..im.w {
            let center = src.get_tc(x, y);
            let mut w = [[[0.0f32; 3]; 3]; 3];
            let mut sum = [0.0f32; 3];
            for j in 0..3 {
                for i in 0..3 {
                    let p = src.get_tc(x - 1 + i as i32, y - 1 + j as i32);
                    for (c, f) in [gd::red, gd::green, gd::blue].iter().enumerate() {
                        w[c][j][i] = if j == 1 && i == 1 {
                            0.5
                        } else {
                            let d = (f(center) as f32 - f(p) as f32).abs();
                            if d != 0.0 { 1.0 / d } else { 1.0 }
                        };
                        sum[c] += w[c][j][i];
                    }
                }
            }
            let mut out = [0.0f32; 3];
            for j in 0..3 {
                for i in 0..3 {
                    let p = src.get_tc(x - 1 + i as i32, y - 1 + j as i32);
                    for (c, f) in [gd::red, gd::green, gd::blue].iter().enumerate() {
                        let wt = if sum[c] != 0.0 { w[c][j][i] / sum[c] } else { w[c][j][i] };
                        out[c] += f(p) as f32 * wt;
                    }
                }
            }
            let a = gd::alpha(src.get_tc(x + 1, y + 1));
            let idx = (y as usize) * (im.w as usize) + x as usize;
            im.px[idx] = argb(a, out[0].clamp(0.0, 255.0) as i32, out[1].clamp(0.0, 255.0) as i32, out[2].clamp(0.0, 255.0) as i32);
        }
    }
}

/// gdImagePixelate: blocks of `size`, each its top-left pixel or (`average`) its mean.
fn pixelate(im: &mut Img, size: i32, average: bool) {
    let src = im.clone();
    let mut y = 0;
    while y < im.h {
        let mut x = 0;
        while x < im.w {
            let c = if average {
                let (mut r, mut g, mut b, mut a, mut n) = (0, 0, 0, 0, 0);
                for cy in 0..size {
                    for cx in 0..size {
                        if x + cx < im.w && y + cy < im.h {
                            let p = src.get_tc(x + cx, y + cy);
                            r += gd::red(p);
                            g += gd::green(p);
                            b += gd::blue(p);
                            a += gd::alpha(p);
                            n += 1;
                        }
                    }
                }
                argb(a / n, r / n, g / n, b / n)
            } else {
                src.get_tc(x, y)
            };
            for cy in 0..size {
                for cx in 0..size {
                    if x + cx < im.w && y + cy < im.h {
                        let idx = ((y + cy) as usize) * (im.w as usize) + (x + cx) as usize;
                        im.px[idx] = c;
                    }
                }
            }
            x += size;
        }
        y += size;
    }
}

/// gdImageScatter: swap pixels with a neighbour a random distance away.
fn scatter(im: &mut Img, sub: i32, plus: i32) {
    let mut seed: u32 = 0x9e37_79b9;
    let mut rand = |lo: i32, hi: i32| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        lo + ((seed >> 8) as i32).rem_euclid((hi - lo).max(0) + 1)
    };
    let src = im.clone();
    for y in 0..im.h {
        for x in 0..im.w {
            let (dx, dy) = (rand(sub, plus), rand(sub, plus));
            let (sx, sy) = (x + dx, y + dy);
            if sx >= 0 && sx < im.w && sy >= 0 && sy < im.h {
                let idx = (y as usize) * (im.w as usize) + x as usize;
                im.px[idx] = src.get_tc(sx, sy);
            }
        }
    }
}

fn imagefilter(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let filter = int(args, 1);
    if args.len() > 6 {
        return Err(ctx.throw("ArgumentCountError", "Wrong parameter count for imagefilter()".into()));
    }
    if !(0..=12).contains(&filter) {
        return Err(value_error(ctx, 2, "filter", "must be one of the IMG_FILTER_* filter constants"));
    }
    // Each filter's own parameter list (zend_parse_parameters' arity messages).
    let (min, max) = match filter {
        2 | 3 | 10 => (3, 3),
        4 => (5, 6),
        11 => (3, 4),
        12 => (4, 5),
        _ => (2, 2),
    };
    let n = args.len();
    if n < min || n > max {
        let (word, expect) = if min == max { ("exactly", min) } else if n < min { ("at least", min) } else { ("at most", max) };
        let plural = if expect == 1 { "" } else { "s" };
        return Err(ctx.throw("ArgumentCountError", format!("imagefilter() expects {word} {expect} argument{plural}, {n} given")));
    }
    if filter == 12 {
        for (i, n) in [(2, 3), (3, 4)] {
            if !(0..=i64::from(i32::MAX)).contains(&int(args, i)) {
                let f = ctx.builtin_name();
                return Err(ctx.throw("ValueError", format!("{f}(): Argument #{n} must be between 0 and {}", i32::MAX)));
            }
        }
    }
    let (a1, a2, a3) = (ci(args, 2), ci(args, 3), ci(args, 4));
    let smooth = arg(args, 2).to_float() as f32;
    let advanced = arg(args, 3).to_bool();
    with(args, 0, |im| match filter {
        0 => per_pixel(im, |r, g, b, _| (255 - r, 255 - g, 255 - b)),
        1 => per_pixel(im, |r, g, b, _| {
            let y = (0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)) as i32;
            (y, y, y)
        }),
        2 => per_pixel(im, |r, g, b, _| (r + a1, g + a1, b + a1)),
        3 => {
            let c = (100.0 - f64::from(a1)) / 100.0;
            let c = c * c;
            per_pixel(im, |r, g, b, _| {
                let m = |v: i32| (((f64::from(v) / 255.0 - 0.5) * c + 0.5) * 255.0).clamp(0.0, 255.0) as i32;
                (m(r), m(g), m(b))
            });
        }
        4 => per_pixel(im, |r, g, b, _| (r + a1, g + a2, b + a3)),
        _ => {
            if !im.tc {
                im.make_truecolor();
            }
            match filter {
                5 => convolve(im, [[-1.0, 0.0, -1.0], [0.0, 4.0, 0.0], [-1.0, 0.0, -1.0]], 1.0, 127.0),
                6 => convolve(im, [[1.5, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, -1.5]], 1.0, 127.0),
                7 => convolve(im, [[1.0, 2.0, 1.0], [2.0, 4.0, 2.0], [1.0, 2.0, 1.0]], 16.0, 0.0),
                8 => selective_blur(im),
                9 => convolve(im, [[-1.0, -1.0, -1.0], [-1.0, 9.0, -1.0], [-1.0, -1.0, -1.0]], 1.0, 0.0),
                10 => convolve(im, [[1.0, 1.0, 1.0], [1.0, smooth, 1.0], [1.0, 1.0, 1.0]], smooth + 8.0, 0.0),
                11 => {
                    if a1 > 1 {
                        pixelate(im, a1, advanced);
                    }
                }
                _ => scatter(im, a1, a2),
            }
        }
    });
    if (filter == 11 && a1 <= 0) || (filter == 12 && a1 >= a2) {
        return Ok(Value::FALSE);
    }
    ok()
}

fn imageconvolution(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    let Ok(rows) = arg(args, 1).into_array() else { unreachable!("checked against the stub") };
    if rows.len() != 3 {
        return Err(value_error(ctx, 2, "matrix", "must be a 3x3 array"));
    }
    let mut m = [[0.0f32; 3]; 3];
    for (j, row) in rows.values().enumerate() {
        let Ok(cols) = row.deref().into_array() else { continue };
        if cols.len() < 3 {
            return Err(value_error(ctx, 2, "matrix", &format!("must be a 3x3 array, matrix[{j}] only has {} elements", cols.len())));
        }
        for (i, v) in cols.values().take(3).enumerate() {
            m[j][i] = v.deref().to_float() as f32;
        }
    }
    let (d, o) = (arg(args, 2).to_float(), arg(args, 3).to_float());
    if !d.is_finite() {
        return Err(value_error(ctx, 3, "divisor", "must be finite"));
    }
    if d == 0.0 {
        return Err(value_error(ctx, 3, "divisor", "must not be 0"));
    }
    if !o.is_finite() {
        return Err(value_error(ctx, 4, "offset", "must be finite"));
    }
    let (div, off) = (d as f32, o as f32);
    with(args, 0, |im| {
        if !im.tc {
            im.make_truecolor();
        }
        convolve(im, m, div, off);
    });
    ok()
}

// ---- output ----

/// Send encoded bytes where `to` says: the output, a stream, or a file.
fn send(ctx: &mut dyn Ctx, func: &str, to: &Value, bytes: &[u8]) -> R {
    match to {
        v if v.is_null() => {
            ctx.echo(bytes);
            ok()
        }
        v if p8_runtime::stream::resource_of(v).is_some() => {
            let r = p8_runtime::stream::resource_of(v).expect("a resource");
            let Some(res) = p8_runtime::stream::resource(&r) else { return Ok(Value::FALSE) };
            if res.closed.get() {
                return Err(ctx.throw("TypeError", format!("{func}(): supplied resource is not a valid stream resource")));
            }
            let mut stream = res.stream.borrow_mut();
            Ok(Value::bool(stream.write(ctx, bytes)?.is_some()))
        }
        other => {
            let path = other.to_php_string().unwrap_or_else(p8_runtime::string::PhpStr::empty);
            if path.as_bytes().contains(&0) {
                return Err(ctx.throw("TypeError", format!("{func}(): Argument #2 ($file) must not contain null bytes")));
            }
            if path.as_bytes().is_empty() {
                return Err(ctx.throw("ValueError", "Path must not be empty".into()));
            }
            let shown = lossy(path.as_bytes());
            if shown.starts_with("php://") {
                let h = ctx.call(&Value::str("fopen"), &mut [Value::str(shown.as_str()), Value::str("wb")])?;
                if h.as_bool() == Some(false) {
                    return Ok(Value::FALSE);
                }
                ctx.call(&Value::str("fwrite"), &mut [h.clone(), Value::str(p8_runtime::string::PhpStr::new(bytes.to_vec()))])?;
                ctx.call(&Value::str("fclose"), &mut [h])?;
                return ok();
            }
            let p = shown.strip_prefix("file://").unwrap_or(&shown);
            match std::fs::write(p, bytes) {
                Ok(()) => ok(),
                Err(e) => {
                    ctx.warn(&format!("{func}({shown}): Failed to open stream: {}", os_error(&e)));
                    Ok(Value::FALSE)
                }
            }
        }
    }
}

/// The output target's own checks, before any work.
fn check_target(ctx: &mut dyn Ctx, func: &str, to: &Value) -> Result<(), Throw> {
    if to.is_null() ||p8_runtime::stream::resource_of(to).is_some() {
        return Ok(());
    }
    let path = to.to_php_string().unwrap_or_else(p8_runtime::string::PhpStr::empty);
    if path.as_bytes().contains(&0) {
        return Err(ctx.throw("TypeError", format!("{func}(): Argument #2 ($file) must not contain null bytes")));
    }
    if path.as_bytes().is_empty() {
        return Err(ctx.throw("ValueError", "Path must not be empty".into()));
    }
    Ok(())
}

fn encode_failed(ctx: &mut dyn Ctx, func: &str) -> R {
    ctx.warn(&format!("{func}(): Could not encode the image"));
    Ok(Value::FALSE)
}

fn imagejpeg(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    check_target(ctx, "imagejpeg", &arg(args, 1))?;
    let q = if given(args, 2) { int(args, 2) } else { -1 };
    if !(-1..=100).contains(&q) {
        return Err(value_error(ctx, 3, "quality", "must be at between -1 and 100"));
    }
    let quality = if q == -1 { 75 } else { q as u8 };
    match read(args, 0, |im| codec::encode_jpeg(im, quality)) {
        Some(bytes) => send(ctx, "imagejpeg", &arg(args, 1), &bytes),
        None => encode_failed(ctx, "imagejpeg"),
    }
}

fn imagepng(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    check_target(ctx, "imagepng", &arg(args, 1))?;
    let q = if given(args, 2) { int(args, 2) } else { -1 };
    if !(-1..=9).contains(&q) {
        return Err(value_error(ctx, 3, "quality", "must be between -1 and 9"));
    }
    match read(args, 0, |im| codec::encode_png(im, q as i32)) {
        Some(bytes) => send(ctx, "imagepng", &arg(args, 1), &bytes),
        None => encode_failed(ctx, "imagepng"),
    }
}

fn imagegif(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    check_target(ctx, "imagegif", &arg(args, 1))?;
    match read(args, 0, codec::encode_gif) {
        Some(bytes) => send(ctx, "imagegif", &arg(args, 1), &bytes),
        None => encode_failed(ctx, "imagegif"),
    }
}

fn imagewebp(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    check_target(ctx, "imagewebp", &arg(args, 1))?;
    let q = if given(args, 2) { int(args, 2) } else { -1 };
    if q < -1 {
        return Err(value_error(ctx, 3, "quality", "must be greater than or equal to -1"));
    }
    if !read(args, 0, |im| im.tc) {
        ctx.warn("imagewebp(): Palette image not supported by webp");
        return ok();
    }
    match read(args, 0, codec::encode_webp) {
        Some(bytes) => send(ctx, "imagewebp", &arg(args, 1), &bytes),
        None => encode_failed(ctx, "imagewebp"),
    }
}

fn imagebmp(ctx: &mut dyn Ctx, args: &mut [Value]) -> R {
    check_target(ctx, "imagebmp", &arg(args, 1))?;
    let bytes = read(args, 0, codec::encode_bmp);
    send(ctx, "imagebmp", &arg(args, 1), &bytes)
}
