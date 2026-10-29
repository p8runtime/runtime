<?php
set_error_handler(function ($n, $s) { echo "[W: $s] "; return true; });
function info($label, $data) {
    $i = getimagesizefromstring($data);
    echo "$label: ", $i ? "{$i[0]}x{$i[1]} type={$i[2]} bits=" . ($i['bits'] ?? '-') . " ch=" . ($i['channels'] ?? '-') . " mime={$i['mime']}" : 'unrecognised', " len>0=" . (strlen($data) > 0 ? 1 : 0) . "\n";
}
function grid($im, $pts) {
    $o = [];
    foreach ($pts as [$x, $y]) $o[] = sprintf("%08x", imagecolorat($im, $x, $y));
    return implode(' ', $o);
}
$tc = imagecreatetruecolor(20, 12);
imagefill($tc, 0, 0, 0xff8040);
imagefilledrectangle($tc, 5, 3, 14, 8, 0x2060c0);
$pts = [[0, 0], [10, 5], [19, 11]];
echo "orig ", grid($tc, $pts), "\n";

// PNG
ob_start(); imagepng($tc); $png = ob_get_clean();
info('png', $png);
$r = imagecreatefromstring($png);
echo "png re ", imagesx($r), 'x', imagesy($r), ' tc=', var_export(imageistruecolor($r), true), ' ', grid($r, $pts), "\n";
foreach ([0, 1, 6, 9] as $lvl) { ob_start(); imagepng($tc, null, $lvl); $d = ob_get_clean(); echo "png level $lvl ok=", var_export(strlen($d) > 50, true), " re=", grid(imagecreatefromstring($d), $pts), "\n"; }
// PNG with alpha
$al = imagecreatetruecolor(4, 2);
imagealphablending($al, false); imagesavealpha($al, true);
imagefill($al, 0, 0, imagecolorallocatealpha($al, 10, 20, 30, 127));
imagesetpixel($al, 1, 0, imagecolorallocatealpha($al, 100, 110, 120, 64));
imagesetpixel($al, 2, 0, imagecolorallocatealpha($al, 200, 210, 220, 0));
ob_start(); imagepng($al); $d = ob_get_clean();
info('png alpha', $d);
$r = imagecreatefromstring($d);
echo "png alpha re tc=", var_export(imageistruecolor($r), true), ' ', grid($r, [[0, 0], [1, 0], [2, 0], [3, 1]]), "\n";
$r2 = imagecreatefromstring($d); echo "  re save-alpha then write again: ";
ob_start(); imagepng($r2); $d2 = ob_get_clean(); $r3 = imagecreatefromstring($d2); echo grid($r3, [[0, 0], [1, 0], [2, 0]]), "\n";
// no savealpha
$al2 = imagecreatetruecolor(2, 1); imagealphablending($al2, false);
imagesetpixel($al2, 0, 0, imagecolorallocatealpha($al2, 100, 110, 120, 64));
ob_start(); imagepng($al2); $d = ob_get_clean(); info('png noalpha', $d);
echo "noalpha re ", grid(imagecreatefromstring($d), [[0, 0]]), "\n";
// palette PNG
$pal = imagecreate(6, 4);
$bg = imagecolorallocate($pal, 255, 255, 255); $red = imagecolorallocate($pal, 255, 0, 0); $blue = imagecolorallocatealpha($pal, 0, 0, 255, 60);
imagefilledrectangle($pal, 1, 1, 3, 2, $red); imagesetpixel($pal, 5, 3, $blue);
ob_start(); imagepng($pal); $d = ob_get_clean(); info('png pal', $d);
$r = imagecreatefromstring($d);
echo "pal re tc=", var_export(imageistruecolor($r), true), ' total=', imagecolorstotal($r), ' ', grid($r, [[0, 0], [2, 2], [5, 3]]), ' transp=', imagecolortransparent($r), "\n";
imagecolortransparent($pal, $bg);
ob_start(); imagepng($pal); $d = ob_get_clean();
$r = imagecreatefromstring($d);
echo "pal transp re total=", imagecolorstotal($r), ' transp=', imagecolortransparent($r), ' ', json_encode(imagecolorsforindex($r, 0)), "\n";

// GIF
ob_start(); imagegif($pal); $gif = ob_get_clean(); info('gif pal', $gif);
$r = imagecreatefromstring($gif);
echo "gif re tc=", var_export(imageistruecolor($r), true), ' total=', imagecolorstotal($r), ' transp=', imagecolortransparent($r), ' ', grid($r, [[0, 0], [2, 2], [5, 3]]), "\n";
// gif with transparency from truecolor alpha
// gif many colours
$many = imagecreatetruecolor(32, 32);
for ($y = 0; $y < 32; $y++) for ($x = 0; $x < 32; $x++) imagesetpixel($many, $x, $y, ($x * 8) << 16 | ($y * 8) << 8 | (($x + $y) * 4));
ob_start(); imagegif($many); $g4 = ob_get_clean(); $r = imagecreatefromstring($g4);
echo "gif many total=", imagecolorstotal($r), ' tc=', var_export(imageistruecolor($r), true), "\n";

// JPEG
foreach ([-1, 0, 50, 100] as $q) {
    ob_start(); imagejpeg($tc, null, $q); $j = ob_get_clean();
    $r = imagecreatefromstring($j);
    $c = imagecolorat($r, 0, 0);
    echo "jpeg q=$q ", imagesx($r), 'x', imagesy($r), ' tc=', var_export(imageistruecolor($r), true), ' near=', var_export(abs(($c >> 16 & 255) - 255) < 12 && abs(($c >> 8 & 255) - 128) < 12 && abs(($c & 255) - 64) < 20, true), "\n";
}
ob_start(); imagejpeg($tc); $j = ob_get_clean(); info('jpeg', $j);
echo "jpeg magic ", bin2hex(substr($j, 0, 4)), "\n";
$ji = getimagesizefromstring($j, $inf); echo "jpeg app0 ", isset($inf['APP0']) ? 'yes' : 'no', "\n";
// jpeg from palette & alpha
ob_start(); imagejpeg($pal); $j = ob_get_clean(); echo "jpeg pal ok ", strlen($j) > 100 ? 'y' : 'n', "\n";
// WEBP
$r = imagecreatefromstring($w);
// BMP
ob_start(); imagebmp($tc); $b = ob_get_clean(); info('bmp', $b);
$r = imagecreatefromstring($b);
echo "bmp re ", imagesx($r), 'x', imagesy($r), ' tc=', var_export(imageistruecolor($r), true), ' ', grid($r, $pts), "\n";
ob_start(); imagebmp($pal); $b = ob_get_clean(); info('bmp pal', $b);
$r = imagecreatefromstring($b); echo "bmp pal re tc=", var_export(imageistruecolor($r), true), ' ', grid($r, [[0, 0], [2, 2]]), "\n";
ob_start(); imagebmp($tc, null, false); $b = ob_get_clean(); info('bmp nc', $b);
// to file and stream
$dir = sys_get_temp_dir();
$f = tempnam($dir, 'gd');
var_dump(imagepng($tc, $f)); echo filesize($f) > 50 ? "file ok\n" : "file empty\n"; info('file', file_get_contents($f));
$r = imagecreatefrompng($f); echo "from png file ", grid($r, $pts), "\n";
var_dump(imagejpeg($tc, $f, 90)); $r = imagecreatefromjpeg($f); echo "from jpeg file ", imagesx($r), "\n";
var_dump(imagegif($tc, $f)); $r = imagecreatefromgif($f); echo "from gif file ", imagesx($r), "\n";
var_dump(imagebmp($tc, $f)); $r = imagecreatefrombmp($f); echo "from bmp file ", imagesx($r), "\n";
$h = fopen($f, 'wb'); var_dump(imagepng($tc, $h)); fclose($h); echo "stream ", filesize($f) > 50 ? 'ok' : 'empty', "\n";
$m = fopen('php://memory', 'w+'); var_dump(imagejpeg($tc, $m)); echo "mem ", ftell($m) > 100 ? 'ok' : 'empty', "\n";
unlink($f);
echo "\n";
// cross-type: wrong decoder
// resolution round trip
$rr = imagecreatetruecolor(4, 4); imageresolution($rr, 300, 150);
ob_start(); imagepng($rr); $d = ob_get_clean(); $r = imagecreatefromstring($d); echo "png res ", json_encode(imageresolution($r)), "\n";
ob_start(); imagejpeg($rr); $d = ob_get_clean(); $r = imagecreatefromstring($d); echo "jpeg res ", json_encode(imageresolution($r)), "\n";
ob_start(); imagebmp($rr); $d = ob_get_clean(); $r = imagecreatefromstring($d); echo "bmp res ", json_encode(imageresolution($r)), "\n";
// returns on closed

