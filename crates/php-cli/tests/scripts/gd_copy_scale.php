<?php
set_error_handler(function ($n, $s) { echo "[W: $s] "; return true; });
// rotate dims
foreach ([[10, 8], [100, 50], [37, 91], [64, 64]] as [$w, $h]) {
    $g = imagecreatetruecolor($w, $h);
    $row = [];
    foreach ([10, 30, 45, 60, 89, 91, 120, 180, 200, 270, 359, -30, 0.5] as $a) {
        $s = imagerotate($g, $a, 0);
        $row[] = "$a:" . imagesx($s) . "x" . imagesy($s);
    }
    echo "rot {$w}x{$h} ", implode(' ', $row), "\n";
}
// rotate pixel check
$g = imagecreatetruecolor(4, 3);
for ($y = 0; $y < 3; $y++) for ($x = 0; $x < 4; $x++) imagesetpixel($g, $x, $y, ($y * 4 + $x + 1) * 0x101010);
foreach ([90, 180, 270] as $a) {
    $s = imagerotate($g, $a, 0);
    $o = [];
    for ($y = 0; $y < imagesy($s); $y++) { for ($x = 0; $x < imagesx($s); $x++) $o[] = dechex(imagecolorat($s, $x, $y) & 0xff); }
    echo "rotpix $a: ", implode(',', $o), "\n";
}
echo "\n";
// resample
$src = imagecreatetruecolor(7, 5);
for ($y = 0; $y < 5; $y++) for ($x = 0; $x < 7; $x++) imagesetpixel($src, $x, $y, imagecolorallocatealpha($src, $x * 30, $y * 50, ($x + $y) * 20, ($x * $y) % 50));
foreach ([[3, 2], [14, 10], [5, 5], [7, 5], [1, 1], [20, 3]] as [$w, $h]) {
    $d = imagecreatetruecolor($w, $h);
    imagealphablending($d, false);
    imagecopyresampled($d, $src, 0, 0, 0, 0, $w, $h, 7, 5);
    $o = [];
    for ($y = 0; $y < $h; $y++) for ($x = 0; $x < $w; $x++) $o[] = dechex(imagecolorat($d, $x, $y));
    echo "resamp {$w}x{$h}: ", implode(',', $o), "\n";
    $d = imagecreatetruecolor($w, $h);
    imagecopyresized($d, $src, 0, 0, 0, 0, $w, $h, 7, 5);
    $o = [];
    for ($y = 0; $y < $h; $y++) for ($x = 0; $x < $w; $x++) $o[] = dechex(imagecolorat($d, $x, $y));
    echo "resiz {$w}x{$h}: ", implode(',', $o), "\n";
}
// scale
foreach ([[3, 2], [14, 10], [7, 5]] as [$w, $h]) {
    $d = imagescale($src, $w, $h);
    $o = [];
    for ($y = 0; $y < $h; $y++) for ($x = 0; $x < $w; $x++) $o[] = dechex(imagecolorat($d, $x, $y));
    echo "scale {$w}x{$h}: ", implode(',', $o), "\n";
}
// blending
$d = imagecreatetruecolor(2, 1);
imagefill($d, 0, 0, imagecolorallocatealpha($d, 100, 100, 100, 50));
imagesetpixel($d, 0, 0, imagecolorallocatealpha($d, 200, 10, 30, 60));
echo "blend: ", dechex(imagecolorat($d, 0, 0)), ' ', dechex(imagecolorat($d, 1, 0)), "\n";
imagealphablending($d, false);
imagesetpixel($d, 0, 0, imagecolorallocatealpha($d, 200, 10, 30, 60));
echo "noblend: ", dechex(imagecolorat($d, 0, 0)), "\n";
// filled rect negative / flipped
$d = imagecreatetruecolor(6, 6);
imagefilledrectangle($d, 4, 4, 1, 1, 0xff);
imagerectangle($d, 0, 0, 5, 5, 0xff00);
$o = '';
for ($y = 0; $y < 6; $y++) { for ($x = 0; $x < 6; $x++) $o .= imagecolorat($d, $x, $y) == 0xff ? 'b' : (imagecolorat($d, $x, $y) == 0xff00 ? 'g' : '.'); $o .= '/'; }
echo "rect: $o\n";
$d = imagecreatetruecolor(9, 9);
$o = '';
for ($y = 0; $y < 9; $y++) { for ($x = 0; $x < 9; $x++) $o .= imagecolorat($d, $x, $y) ? '#' : '.'; $o .= '/'; }
$d = imagecreatetruecolor(9, 9);
$o = '';
for ($y = 0; $y < 9; $y++) { for ($x = 0; $x < 9; $x++) $o .= imagecolorat($d, $x, $y) ? '#' : '.'; $o .= '/'; }


// edge behaviour of imagescale bilinear
$src = imagecreatetruecolor(3, 3);
$vals = [[0x100000, 0x200000, 0x400000], [0x001000, 0x002000, 0x004000], [0x000010, 0x000020, 0x000040]];
for ($y = 0; $y < 3; $y++) for ($x = 0; $x < 3; $x++) imagesetpixel($src, $x, $y, $vals[$y][$x]);
$d = imagescale($src, 6, 6);
for ($y = 0; $y < 6; $y++) { for ($x = 0; $x < 6; $x++) printf("%06x ", imagecolorat($d, $x, $y)); echo "\n"; }
echo "--\n";
$d = imagescale($src, 5, 5);
for ($y = 0; $y < 5; $y++) { for ($x = 0; $x < 5; $x++) printf("%06x ", imagecolorat($d, $x, $y)); echo "\n"; }
echo "-- 2\n";
$d = imagescale($src, 2, 2);
for ($y = 0; $y < 2; $y++) { for ($x = 0; $x < 2; $x++) printf("%06x ", imagecolorat($d, $x, $y)); echo "\n"; }
// copyresampled alpha / rounding
$s = imagecreatetruecolor(2, 2);
imagesetpixel($s, 0, 0, 0x00ff0000); imagesetpixel($s, 1, 0, 0x0000ff00); imagesetpixel($s, 0, 1, 0x000000ff); imagesetpixel($s, 1, 1, 0x00ffffff);
$d = imagecreatetruecolor(1, 1);
imagecopyresampled($d, $s, 0, 0, 0, 0, 1, 1, 2, 2);
printf("avg %08x\n", imagecolorat($d, 0, 0));
$s2 = imagecreatetruecolor(3, 1);
imagesetpixel($s2, 0, 0, 0x01010101); imagesetpixel($s2, 1, 0, 0x02020202); imagesetpixel($s2, 2, 0, 0x05050505);
$d = imagecreatetruecolor(2, 1);
imagealphablending($d, false);
imagecopyresampled($d, $s2, 0, 0, 0, 0, 2, 1, 3, 1);
printf("avg2 %08x %08x\n", imagecolorat($d, 0, 0), imagecolorat($d, 1, 0));
// opaque
$s3 = imagecreatetruecolor(3, 1);
imagesetpixel($s3, 0, 0, 0x000a0a0a); imagesetpixel($s3, 1, 0, 0x00141414); imagesetpixel($s3, 2, 0, 0x001e1e1e);
$d = imagecreatetruecolor(2, 1);
imagecopyresampled($d, $s3, 0, 0, 0, 0, 2, 1, 3, 1);
printf("avg3 %08x %08x\n", imagecolorat($d, 0, 0), imagecolorat($d, 1, 0));


function pat($w, $h) {
    $g = imagecreatetruecolor($w, $h);
    for ($y = 0; $y < $h; $y++) for ($x = 0; $x < $w; $x++) imagesetpixel($g, $x, $y, (($x * 13) % 256) << 16 | (($y * 29) % 256) << 8 | (($x * $y) % 256));
    return $g;
}
$g = pat(20, 15);
foreach ([-90, 180, -180, 720, 90.0] as $a) {
    $s = imagerotate($g, $a, 0x7f00ff00);
    $h = '';
    for ($y = 0; $y < imagesy($s); $y++) for ($x = 0; $x < imagesx($s); $x++) $h .= imagecolorat($s, $x, $y) . ',';
    echo "rot $a ", imagesx($s), 'x', imagesy($s), ' ', md5($h), ' ', imagecolorat($s, imagesx($s) >> 1, imagesy($s) >> 1), "\n";
}
// palette rotate
$p = imagecreate(5, 4);
$c1 = imagecolorallocate($p, 255, 0, 0); $c2 = imagecolorallocate($p, 0, 255, 0);
imagefilledrectangle($p, 0, 0, 2, 3, $c2);
$s = imagerotate($p, 90, $c1);
echo "palrot ", imagesx($s), 'x', imagesy($s), ' ', var_export(imageistruecolor($s), true), ' ', imagecolorat($s, 0, 0), "\n";
// flip
$f = pat(5, 4);
foreach ([IMG_FLIP_HORIZONTAL, IMG_FLIP_VERTICAL, IMG_FLIP_BOTH] as $m) {
    $c = pat(5, 4); imageflip($c, $m);
    echo "flip $m ", imagecolorat($c, 0, 0), ' ', imagecolorat($c, 4, 3), ' ', imagecolorat($c, 1, 2), "\n";
}
// copy
$d = imagecreatetruecolor(10, 10);
imagefill($d, 0, 0, 0xff00ff);
imagecopy($d, $f, 2, 2, 1, 1, 3, 3);
echo "copy ", imagecolorat($d, 2, 2), ' ', imagecolorat($d, 4, 4), ' ', imagecolorat($d, 5, 5), "\n";
$d2 = imagecreatetruecolor(10, 10);
imagecopymerge($d2, $f, 0, 0, 0, 0, 5, 4, 50);
echo "merge ", imagecolorat($d2, 1, 1), ' ', imagecolorat($d2, 4, 3), "\n";
imagecopymergegray($d2, $f, 0, 0, 0, 0, 5, 4, 50);
echo "mergegray ", imagecolorat($d2, 1, 1), ' ', imagecolorat($d2, 4, 3), "\n";
// copy with alpha blending
$a = imagecreatetruecolor(3, 1);
imagefill($a, 0, 0, 0x336699);
$b = imagecreatetruecolor(3, 1);
imagealphablending($b, false);
imagesetpixel($b, 0, 0, 0x40ff0000); imagesetpixel($b, 1, 0, 0x7f00ff00); imagesetpixel($b, 2, 0, 0x20112233);
imagecopy($a, $b, 0, 0, 0, 0, 3, 1);
printf("copyalpha %08x %08x %08x\n", imagecolorat($a, 0, 0), imagecolorat($a, 1, 0), imagecolorat($a, 2, 0));
$a = imagecreatetruecolor(3, 1);
imagefill($a, 0, 0, 0x336699);
imagealphablending($a, false);
imagecopy($a, $b, 0, 0, 0, 0, 3, 1);
printf("copyalpha-noblend %08x %08x %08x\n", imagecolorat($a, 0, 0), imagecolorat($a, 1, 0), imagecolorat($a, 2, 0));
// palette copy into truecolor, transparent
$pal = imagecreate(3, 1);
$k = imagecolorallocate($pal, 10, 20, 30); $t = imagecolorallocate($pal, 200, 100, 50);
imagesetpixel($pal, 0, 0, $k); imagesetpixel($pal, 1, 0, $t); imagesetpixel($pal, 2, 0, $k);
imagecolortransparent($pal, $t);
$tc = imagecreatetruecolor(3, 1); imagefill($tc, 0, 0, 0xabcdef);
imagecopy($tc, $pal, 0, 0, 0, 0, 3, 1);
printf("palcopy %08x %08x %08x\n", imagecolorat($tc, 0, 0), imagecolorat($tc, 1, 0), imagecolorat($tc, 2, 0));
// truecolor into palette
$pp = imagecreate(3, 1); imagecolorallocate($pp, 0, 0, 0);
imagecopy($pp, $tc, 0, 0, 0, 0, 3, 1);
echo "tc2pal ", imagecolorstotal($pp), ' ', imagecolorat($pp, 0, 0), ' ', imagecolorat($pp, 1, 0), "\n";
// fill / border
$f = imagecreatetruecolor(6, 6);
imagerectangle($f, 1, 1, 4, 4, 0xffffff);
imagefill($f, 2, 2, 0xff0000);
imagefill($f, 0, 0, 0x0000ff);
$o = '';
for ($y = 0; $y < 6; $y++) { for ($x = 0; $x < 6; $x++) $o .= dechex(imagecolorat($f, $x, $y)) . ' '; $o .= '/'; }
echo "fill $o\n";
$f = imagecreatetruecolor(6, 6);
imagerectangle($f, 1, 1, 4, 4, 0xffffff);
imagefilltoborder($f, 2, 2, 0xffffff, 0xff0000);
$o = '';
for ($y = 0; $y < 6; $y++) { for ($x = 0; $x < 6; $x++) $o .= dechex(imagecolorat($f, $x, $y)) . ' '; $o .= '/'; }
echo "ftb $o\n";
// filters
foreach ([IMG_FILTER_NEGATE => [], IMG_FILTER_GRAYSCALE => [], IMG_FILTER_BRIGHTNESS => [30], IMG_FILTER_CONTRAST => [20], IMG_FILTER_COLORIZE => [10, 20, 30], IMG_FILTER_EDGEDETECT => [], IMG_FILTER_EMBOSS => [], IMG_FILTER_GAUSSIAN_BLUR => [], IMG_FILTER_MEAN_REMOVAL => [], IMG_FILTER_SMOOTH => [5], IMG_FILTER_PIXELATE => [2], 99 => []] as $fl => $ar) {
    $c = pat(6, 5);
    try { $r = imagefilter($c, $fl, ...$ar); } catch (Throwable $e) { echo "filter $fl ", $e->getMessage(), "\n"; continue; }
    $h = '';
    for ($y = 0; $y < 5; $y++) for ($x = 0; $x < 6; $x++) $h .= imagecolorat($c, $x, $y) . ',';
    echo "filter $fl ", var_export($r, true), ' ', md5($h), "\n";
}
$c = pat(6, 5);
imagegammacorrect($c, 1.0, 2.2);
$h = '';
for ($y = 0; $y < 5; $y++) for ($x = 0; $x < 6; $x++) $h .= imagecolorat($c, $x, $y) . ',';
echo "gamma ", md5($h), "\n";
$c = pat(6, 5);
$r = imageconvolution($c, [[1, 2, 1], [2, 4, 2], [1, 2, 1]], 16, 0);
$h = '';
for ($y = 0; $y < 5; $y++) for ($x = 0; $x < 6; $x++) $h .= imagecolorat($c, $x, $y) . ',';
echo "conv ", var_export($r, true), ' ', md5($h), "\n";
// cropauto
$c = imagecreatetruecolor(10, 10); imagefilledrectangle($c, 3, 2, 6, 7, 0xff0000);
foreach ([IMG_CROP_DEFAULT, IMG_CROP_BLACK, IMG_CROP_WHITE, IMG_CROP_SIDES, IMG_CROP_TRANSPARENT] as $m) {
    $r = imagecropauto($c, $m);
    echo "cropauto $m ", $r ? imagesx($r) . 'x' . imagesy($r) : 'false', "\n";
}
$r = imagecropauto($c, IMG_CROP_THRESHOLD, 0.5, 0);
echo "cropauto thr ", $r ? imagesx($r) . 'x' . imagesy($r) : 'false', "\n";
try { imagecropauto($c, 9); } catch (Throwable $e) { echo $e->getMessage(), "\n"; }
// gif palette -> scale/crop keep types
$g = imagecreate(4, 4); imagecolorallocate($g, 1, 2, 3);
echo "scale pal tc? ", var_export(imageistruecolor(imagescale($g, 8, 8)), true), "\n";
// interpolation influences scale
$s = pat(9, 9);
$r = imagescale($s, 13, 7);
$h = '';
for ($y = 0; $y < 7; $y++) for ($x = 0; $x < 13; $x++) $h .= imagecolorat($r, $x, $y) . ',';
echo "scale 13x7 ", md5($h), "\n";
echo "scale ratio ", imagesx($r = imagescale($s, 4)), 'x', imagesy($r), "\n";
echo "scale ratio ", imagesx($r = imagescale($pat = pat(30, 7), 10)), 'x', imagesy($r), "\n";

