<?php
// Images: headers built here, identified as finfo does.
$flags = [FILEINFO_NONE, FILEINFO_MIME_TYPE, FILEINFO_MIME, FILEINFO_MIME_ENCODING, FILEINFO_EXTENSION];

function show(string $label, string $data): void {
    global $flags;
    $row = [];
    foreach ($flags as $f) {
        $row[] = (new finfo($f))->buffer($data);
    }
    echo str_pad($label, 14), ' ', implode(' | ', $row), "\n";
}

function png(int $w, int $h, int $depth, int $color, int $interlace = 0): string {
    $ihdr = pack('NNCCCCC', $w, $h, $depth, $color, 0, 0, $interlace);
    return "\x89PNG\r\n\x1a\n" . pack('N', 13) . 'IHDR' . $ihdr . pack('N', crc32('IHDR' . $ihdr)) . pack('N', 0) . 'IEND' . pack('N', crc32('IEND'));
}

function be16(int $n): string { return pack('n', $n); }

function jpeg(array $segments): string {
    $out = "\xff\xd8";
    foreach ($segments as [$marker, $body]) {
        $out .= "\xff" . chr($marker) . be16(strlen($body) + 2) . $body;
    }
    return $out . "\xff\xd9";
}

$jfif = "JFIF\x00\x01\x01\x01" . be16(72) . be16(72) . "\x00\x00";
$sof = "\x08" . be16(60) . be16(80) . "\x03\x01\x22\x00\x02\x11\x01\x03\x11\x01";

$tiff_le = "II*\x00\x08\x00\x00\x00" . pack('v', 3)
    . pack('vvVV', 0x100, 3, 1, 30) . pack('vvVV', 0x101, 3, 1, 20) . pack('vvVV', 0x112, 3, 1, 1) . pack('V', 0);
$tiff_be = "MM\x00*\x00\x00\x00\x08" . pack('n', 3)
    . pack('nnNN', 0x100, 3, 1, 30 << 16) . pack('nnNN', 0x101, 3, 1, 20 << 16) . pack('nnNN', 0x112, 3, 1, 6 << 16) . pack('N', 0);

$bmp = 'BM' . pack('VvvV', 70, 0, 0, 54) . pack('VVVvvVVVVVV', 40, 4, 4, 1, 24, 0, 16, 2835, 2835, 0, 0) . str_repeat("\x00", 16);
$ico = pack('vvv', 0, 1, 1) . pack('CCCCvvVV', 16, 16, 0, 0, 1, 32, 40, 22) . str_repeat("\x00", 40);
$cur = pack('vvv', 0, 2, 1) . pack('CCCCvvVV', 16, 16, 0, 0, 1, 1, 40, 22) . str_repeat("\x00", 40);

$webp_vp8 = 'RIFF' . pack('V', 30) . 'WEBPVP8 ' . pack('V', 18) . "\x30\x01\x00\x9d\x01\x2a" . pack('vv', 50, 40) . str_repeat("\x00", 8);
$webp_lossless = 'RIFF' . pack('V', 26) . 'WEBPVP8L' . pack('V', 14) . "\x2f\x31\x00\x00\x00" . str_repeat("\x00", 9);
$webp_ext = 'RIFF' . pack('V', 30) . 'WEBPVP8X' . pack('V', 10) . "\x10\x00\x00\x00\x31\x00\x00\x27\x00\x00";

function ftyp(string $brand, string $compat = ''): string {
    $body = $brand . "\x00\x00\x00\x00" . $compat;
    return pack('N', 8 + strlen($body)) . 'ftyp' . $body;
}

$samples = [
    'png rgba'    => png(16, 16, 8, 6),
    'png rgb'     => png(100, 50, 8, 2),
    'png gray'    => png(3, 3, 8, 0),
    'png palette' => png(640, 480, 8, 3),
    'png ga'      => png(5, 6, 16, 4),
    'png adam7'   => png(7, 7, 8, 6, 1),
    'png short'   => "\x89PNG\r\n\x1a\n",
    'gif89a'      => "GIF89a" . pack('vv', 50, 40) . "\xf7\x00\x00",
    'gif87a'      => "GIF87a" . pack('vv', 320, 200) . "\x00\x00\x00",
    'jpeg jfif'   => jpeg([[0xe0, $jfif], [0xc0, $sof]]),
    'jpeg prog'   => jpeg([[0xe0, $jfif], [0xc2, $sof]]),
    'jpeg comment' => jpeg([[0xe0, $jfif], [0xfe, 'made by test'], [0xc0, $sof]]),
    'jpeg exif'   => jpeg([[0xe1, "Exif\x00\x00" . $tiff_le], [0xc0, $sof]]),
    'jpeg exif be' => jpeg([[0xe0, $jfif], [0xe1, "Exif\x00\x00" . $tiff_be], [0xdb, str_repeat("\x01", 65)], [0xc0, $sof]]),
    'jpeg bare'   => "\xff\xd8\xff\xdb\x00\x03\x00",
    'tiff le'     => $tiff_le,
    'tiff be'     => $tiff_be,
    'bmp'         => $bmp,
    'ico'         => $ico,
    'cur'         => $cur,
    'webp vp8'    => $webp_vp8,
    'webp vp8l'   => $webp_lossless,
    'webp vp8x'   => $webp_ext,
    'avif'        => ftyp('avif', 'mif1miaf'),
    'avif seq'    => ftyp('avis', 'avismsf1'),
    'heic'        => ftyp('heic', 'mif1heic'),
    'heif'        => ftyp('mif1', 'heic'),
    'psd'         => "8BPS\x00\x01" . str_repeat("\x00", 6) . be16(3) . pack('NN', 40, 30) . be16(8) . be16(3) . str_repeat("\x00", 10),
    'svg'         => '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
    'xpm'         => "/* XPM */\nstatic char *a[] = {\n\"1 1 1 1\",\n\". c #000000\",\n\".\"};\n",
    'pbm'         => "P1\n2 2\n0 1\n1 0\n",
    'ppm'         => "P6\n2 2\n255\n" . str_repeat("\x00", 12),
];
foreach ($samples as $label => $data) {
    show($label, $data);
}
