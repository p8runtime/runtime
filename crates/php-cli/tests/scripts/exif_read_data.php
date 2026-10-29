<?php
set_error_handler(function ($n, $s) { echo "[W: ", preg_replace(['/\(.*?\)/', '~/\S*exif\w+~'], ['(FILE)', 'TMP'], $s, 1), "] "; return true; });

// A JPEG that is only its Exif block, a comment and a frame header, built by hand.
function jpeg_with_exif(bool $le): string {
    $u16 = fn ($v) => pack($le ? 'v' : 'n', $v);
    $u32 = fn ($v) => pack($le ? 'V' : 'N', $v);
    $rat = fn ($n, $d) => $u32($n) . $u32($d);
    $srat = fn ($n, $d) => $u32($n & 0xffffffff) . $u32($d);
    // [tag, type, count, bytes]
    $ifd0 = [
        [0x0102, 3, 3, $u16(8) . $u16(8) . $u16(8)],
        [0x010F, 2, 13, "Acme Cameras\0"],
        [0x0110, 2, 4, "X-1\0"],
        [0x0112, 3, 1, $u16(6)],
        [0x011A, 5, 1, $rat(72, 1)],
        [0x0132, 2, 20, "2020:01:02 03:04:05\0"],
        [0x8298, 2, 14, "Photog\0Editor\0"],
        [0x8769, 4, 1, 'EXIF'],
        [0x8825, 4, 1, 'GPS'],
    ];
    $exif = [
        [0x829A, 5, 1, $rat(1, 125)],
        [0x829D, 5, 1, $rat(28, 10)],
        [0x8827, 3, 1, $u16(200)],
        [0x9000, 7, 4, '0230'],
        [0x9003, 2, 20, "2019:12:31 23:59:58\0"],
        [0x9201, 10, 1, $srat(-3, 1)],
        [0x9206, 5, 1, $rat(5, 1)],
        [0x9286, 7, 13, "ASCII\0\0\0hello"],
        [0x920A, 5, 1, $rat(50, 1)],
        [0xA002, 4, 1, $u32(640)],
        [0xA003, 4, 1, $u32(480)],
        [0xA005, 4, 1, 'IOP'],
        [0xC4A5, 7, 3, 'abc'],
    ];
    $iop = [[0x0001, 2, 4, "R98\0"]];
    $gps = [[0x0001, 2, 2, "N\0"], [0x0002, 5, 3, $rat(10, 1) . $rat(20, 1) . $rat(30, 1)]];
    $ifd1 = [[0x0103, 3, 1, $u16(6)], [0x0201, 4, 1, 'THUMB'], [0x0202, 4, 1, $u32(4)]];
    $size = fn ($e) => 2 + 12 * count($e) + 4;
    $at = [];
    $off = 8;
    foreach (['ifd0' => $ifd0, 'exif' => $exif, 'iop' => $iop, 'gps' => $gps, 'ifd1' => $ifd1] as $name => $e) {
        $at[$name] = $off;
        $off += $size($e);
    }
    $thumb = $off;
    $data = "\xff\xd8\xff\xd9"; // the thumbnail's bytes
    $dataAt = $off + strlen($data);
    $build = function ($entries, $next) use (&$data, &$dataAt, $u16, $u32, $at, $thumb) {
        $out = $u16(count($entries));
        foreach ($entries as [$tag, $type, $count, $bytes]) {
            if ($bytes === 'EXIF') $bytes = $u32($at['exif']);
            elseif ($bytes === 'GPS') $bytes = $u32($at['gps']);
            elseif ($bytes === 'IOP') $bytes = $u32($at['iop']);
            elseif ($bytes === 'THUMB') $bytes = $u32($thumb);
            if (strlen($bytes) > 4) {
                $value = $u32($dataAt);
                $data .= $bytes;
                $dataAt += strlen($bytes);
            } else {
                $value = str_pad($bytes, 4, "\0");
            }
            $out .= $u16($tag) . $u16($type) . $u32($count) . $value;
        }
        return $out . $u32($next);
    };
    $tiff = ($le ? 'II' : 'MM') . $u16(42) . $u32(8);
    $tiff .= $build($ifd0, $at['ifd1']) . $build($exif, 0) . $build($iop, 0) . $build($gps, 0) . $build($ifd1, 0) . $data;
    $app1 = "Exif\0\0" . $tiff;
    $sof = "\x08" . pack('n', 480) . pack('n', 640) . "\x03\x01\x22\x00\x02\x11\x01\x03\x11\x01";
    return "\xff\xd8" . "\xff\xe1" . pack('n', strlen($app1) + 2) . $app1
        . "\xff\xfe" . pack('n', 8) . "a note"
        . "\xff\xc0" . pack('n', strlen($sof) + 2) . $sof
        . "\xff\xda" . pack('n', 2);
}

function show($label, $v) {
    echo "== $label\n";
    if (is_array($v)) {
        unset($v['FileName'], $v['FileDateTime']);
        if (isset($v['FILE'])) {
            unset($v['FILE']['FileName'], $v['FILE']['FileDateTime']);
        }
        foreach ($v as $k => $x) {
            echo "$k = ", json_encode(array_map(fn ($v) => is_string($v) && !preg_match('//u', $v) ? 'hex:' . bin2hex($v) : $v, (array) $x)), "\n";
        }
    } else {
        var_dump($v);
    }
}

$f = tempnam(sys_get_temp_dir(), 'exif');
foreach ([false, true] as $le) {
    file_put_contents($f, jpeg_with_exif($le));
    $name = $le ? 'little-endian' : 'big-endian';
    show("$name flat", exif_read_data($f));
    if (!$le) {
        show("$name sections", exif_read_data($f, 'EXIF,GPS', true));
        show("$name with thumbnail", exif_read_data($f, null, false, true));
        var_dump(exif_read_data($f, 'MAKERNOTE'));
        $w = $h = $t = null;
        var_dump(bin2hex(exif_thumbnail($f, $w, $h, $t)), $w, $h, $t);
        $h = fopen($f, 'rb');
        fseek($h, 5);
        var_dump(count(exif_read_data($h)) > 5, ftell($h));
        fclose($h);
    }
}
// Not images, or not EXIF-bearing ones.
file_put_contents($f, "\xff\xd8\xff\xda\x00\x02");
show('jpeg without exif', exif_read_data($f));
file_put_contents($f, 'plain text here, not an image at all');
var_dump(exif_read_data($f));
file_put_contents($f, 'x');
var_dump(exif_read_data($f));
var_dump(exif_read_data('/nonexistent/file.jpg'));
try { exif_read_data(''); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }

// exif_imagetype and exif_tagname.
foreach (["\xff\xd8\xff\xe0", "GIF89a....", "\x89PNG\r\n\x1a\n....", "BM....", "II*\0....", "MM\0*....", "RIFF....WEBPVP8 ", "nothing"] as $head) {
    file_put_contents($f, $head);
    var_dump(exif_imagetype($f));
}
var_dump(exif_imagetype('/nonexistent/file.jpg'));
var_dump(exif_tagname(0x010F), exif_tagname(0x9003), exif_tagname(0xA005), exif_tagname(0x0001), exif_tagname(0xFFFF), exif_tagname(0x7777), exif_tagname(-5));
unlink($f);
