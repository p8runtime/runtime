<?php
set_error_handler(function ($n, $s) { echo "[W: $s] "; return true; });
function rt($im, $fn) { ob_start(); $fn($im); $d = ob_get_clean(); return imagecreatefromstring($d); }
// gif totals for n colours
foreach ([1, 2, 3, 4, 5, 8, 9, 17, 200, 256] as $n) {
    $p = imagecreate(16, 16);
    $idx = [];
    for ($i = 0; $i < $n; $i++) $idx[] = imagecolorallocate($p, $i, 255 - ($i % 256), ($i * 7) % 256);
    for ($i = 0; $i < 256; $i++) imagesetpixel($p, $i % 16, intdiv($i, 16), $idx[$i % $n]);
    $r = rt($p, 'imagegif');
    $q = rt($p, 'imagepng');
    echo "n=$n gif total=", imagecolorstotal($r), " png total=", imagecolorstotal($q), "\n";
}
// png palette ordering with various alphas / transparent index
$p = imagecreate(4, 1);
$a = imagecolorallocate($p, 10, 0, 0); $b = imagecolorallocate($p, 20, 0, 0); $c = imagecolorallocate($p, 30, 0, 0); $d = imagecolorallocate($p, 40, 0, 0);
for ($i = 0; $i < 4; $i++) imagesetpixel($p, $i, 0, $i);
imagecolortransparent($p, 2);
$r = rt($p, 'imagepng');
echo "transparent=2 -> ", imagecolortransparent($r), ' ', json_encode(array_map(fn($i) => imagecolorsforindex($r, $i)['red'], range(0, imagecolorstotal($r) - 1))), ' px ', imagecolorat($r, 0, 0), imagecolorat($r, 1, 0), imagecolorat($r, 2, 0), imagecolorat($r, 3, 0), "\n";
$p = imagecreate(4, 1);
$a = imagecolorallocatealpha($p, 10, 0, 0, 100); $b = imagecolorallocate($p, 20, 0, 0); $c = imagecolorallocatealpha($p, 30, 0, 0, 50); $d = imagecolorallocate($p, 40, 0, 0);
for ($i = 0; $i < 4; $i++) imagesetpixel($p, $i, 0, $i);
imagesavealpha($p, true);
$r = rt($p, 'imagepng');
echo "alphas -> ", imagecolortransparent($r), ' ', json_encode(array_map(fn($i) => [imagecolorsforindex($r, $i)['red'], imagecolorsforindex($r, $i)['alpha']], range(0, imagecolorstotal($r) - 1))), "\n";
// gif of tc with transparent colour
$t = imagecreatetruecolor(4, 1);
imagefill($t, 0, 0, 0x102030); imagesetpixel($t, 1, 0, 0xaa0000); imagesetpixel($t, 2, 0, 0x102030);
imagecolortransparent($t, 0x102030);
$r = rt($t, 'imagegif');
echo "tc+transp gif -> transp=", imagecolortransparent($r), ' total=', imagecolorstotal($r), ' px ', imagecolorat($r, 0, 0), imagecolorat($r, 1, 0), imagecolorat($r, 2, 0), imagecolorat($r, 3, 0), "\n";
$r = rt($t, 'imagepng');
echo "tc+transp png -> tc=", var_export(imageistruecolor($r), true), ' transp=', imagecolortransparent($r), ' ', dechex(imagecolorat($r, 0, 0)), "\n";
$t2 = imagecreatetruecolor(4, 1); imagealphablending($t2, false); imagesavealpha($t2, true);
imagefill($t2, 0, 0, 0x7f102030);
$r = rt($t2, 'imagegif'); echo "tc alpha127 gif -> transp=", imagecolortransparent($r), ' total=', imagecolorstotal($r), "\n";
// truecolortopalette
$t = imagecreatetruecolor(10, 10);
for ($y = 0; $y < 10; $y++) for ($x = 0; $x < 10; $x++) imagesetpixel($t, $x, $y, ($x * 25) << 16 | ($y * 25) << 8);
var_dump(imagetruecolortopalette($t, false, 8));
echo "tc2pal total=", imagecolorstotal($t), ' tc=', var_export(imageistruecolor($t), true), "\n";
try { imagetruecolortopalette($t, true, 0); } catch (Throwable $e) { echo $e->getMessage(), "\n"; }
try { imagetruecolortopalette($t, true, 257); } catch (Throwable $e) { echo $e->getMessage(), "\n"; }
// bmp variants
$p = imagecreate(5, 3); $c1 = imagecolorallocate($p, 1, 2, 3); $c2 = imagecolorallocate($p, 4, 5, 6); imagesetpixel($p, 2, 1, $c2);
$r = rt($p, 'imagebmp'); echo "bmp pal: tc=", var_export(imageistruecolor($r), true), ' total=', imagecolorstotal($r), ' ', imagecolorat($r, 2, 1), "\n";
$r = rt($p, fn($i) => imagebmp($i, null, false)); echo "bmp pal nc: tc=", var_export(imageistruecolor($r), true), ' total=', imagecolorstotal($r), "\n";
// loaded png properties
$t = imagecreatetruecolor(2, 2); imagealphablending($t, false); imagesavealpha($t, true); imagefill($t, 0, 0, 0x40ff0000);
$r = rt($t, 'imagepng');
echo "loaded rgba png: ", dechex(imagecolorat($r, 0, 0)), "\n";
imagealphablending($r, true);
$bgi = imagecreatetruecolor(2, 2); imagefill($bgi, 0, 0, 0x0000ff);
imagecopy($bgi, $r, 0, 0, 0, 0, 2, 2);
echo "blend over: ", dechex(imagecolorat($bgi, 0, 0)), "\n";
// does loaded png keep alpha when re-saved without savealpha
$r2 = rt($r, 'imagepng'); echo "resave no savealpha call: ", dechex(imagecolorat($r2, 0, 0)), "\n";
// jpeg of alpha tc: alpha ignored?
$t = imagecreatetruecolor(2, 1); imagealphablending($t, false); imagefill($t, 0, 0, 0x7fff0000);
$r = rt($t, 'imagejpeg'); $c = imagecolorat($r, 0, 0); echo "jpeg alpha: ", ($c >> 16 & 255) > 200 ? 'red kept' : 'dropped', ' alpha=', $c >> 24, "\n";
// gray png / 16 bit
echo "done\n";

