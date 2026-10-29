<?php
// getimagesize/getimagesizefromstring on headers built here: GIF, PNG, JPEG
// (with APP markers), BMP, WebP (VP8, VP8L, VP8X), ICO, PSD, TIFF, WBMP, XBM.
$jpeg = "\xFF\xD8\xFF\xE0" . pack('n', 16) . "JFIF\0\x01\x01\0\0\x01\0\x01\0\0" . "\xFF\xE1" . pack('n', 6) . "Exif" . "\xFF\xC0" . pack('n', 17) . "\x08" . pack('n', 30) . pack('n', 40) . "\x03" . str_repeat("\0", 9) . "\xFF\xD9";
$images = [
    'gif' => "GIF89a" . pack('v', 10) . pack('v', 20) . "\xF7\0\0",
    'png' => "\x89PNG\r\n\x1a\n" . pack('N', 13) . "IHDR" . pack('N', 640) . pack('N', 480) . "\x08\x06\0\0\0",
    'jpeg' => $jpeg,
    'bmp' => "BM" . pack('V', 0) . pack('V', 0) . pack('V', 54) . pack('V', 40) . pack('V', 3) . pack('l', -5) . pack('v', 1) . pack('v', 24) . str_repeat("\0", 24),
    'webp lossy' => "RIFF" . pack('V', 30) . "WEBPVP8 " . pack('V', 10) . "\0\0\0\x9d\x01\x2a" . pack('v', 7) . pack('v', 9),
    'webp lossless' => "RIFF" . pack('V', 30) . "WEBPVP8L" . pack('V', 10) . "\x2f\x04\x40\x02\x00" . "\0\0\0",
    'webp extended' => "RIFF" . pack('V', 30) . "WEBPVP8X" . pack('V', 10) . "\0\0\0\0" . "\x0f\0\0\x1f\0\0",
    'ico' => "\0\0\x01\0\x02\0" . "\x10\x10\0\0\x01\0\x08\0" . str_repeat("\0", 8) . "\x20\x20\0\0\x01\0\x20\0" . str_repeat("\0", 8),
    'psd' => "8BPS\0\x01" . str_repeat("\0", 6) . "\0\x03" . pack('N', 77) . pack('N', 88) . "\0\x08\0\x03",
    'tiff' => "II*\0" . pack('V', 8) . pack('v', 2) . pack('vvVV', 0x100, 3, 1, 123) . pack('vvVV', 0x101, 4, 1, 456) . pack('V', 0),
    'wbmp' => "\0\0\x81\x00\x0a",
    'xbm' => "#define test_width 16\n#define test_height 7\nstatic char test_bits[] = {};\n",
    'garbage' => "not an image at all",
];
foreach ($images as $name => $bytes) {
    $info = null;
    echo $name, ': ', json_encode(getimagesizefromstring($bytes, $info)), ' ', json_encode(array_map('bin2hex', $info)), "\n";
}
$f = __DIR__ . '/getimagesize_formats.tmp';
file_put_contents($f, $images['png']);
var_dump(getimagesize($f)[3]);
unlink($f);
var_dump(@getimagesize(__DIR__ . '/no-such-file.png'));
foreach ([IMAGETYPE_GIF, IMAGETYPE_JPEG, IMAGETYPE_PNG, IMAGETYPE_SWF, IMAGETYPE_WBMP, IMAGETYPE_ICO, IMAGETYPE_WEBP, IMAGETYPE_AVIF, 99] as $t) {
    echo $t, ' ', image_type_to_mime_type($t), ' ', var_export(image_type_to_extension($t), true), ' ', var_export(image_type_to_extension($t, false), true), "\n";
}
