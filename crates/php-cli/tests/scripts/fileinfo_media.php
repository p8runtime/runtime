<?php
// Audio, video, fonts and documents.
$flags = [FILEINFO_NONE, FILEINFO_MIME_TYPE, FILEINFO_MIME, FILEINFO_MIME_ENCODING, FILEINFO_EXTENSION];

function show(string $label, string $data): void {
    global $flags;
    $row = [];
    foreach ($flags as $f) {
        $row[] = (new finfo($f))->buffer($data);
    }
    echo str_pad($label, 14), ' ', implode(' | ', $row), "\n";
}

function box(string $type, string $body): string {
    return pack('N', 8 + strlen($body)) . $type . $body;
}

function ftyp(string $brand, string $compat = ''): string {
    return box('ftyp', $brand . "\x00\x00\x00\x00" . $compat);
}

function riff(string $form, string $body): string {
    return 'RIFF' . pack('V', 4 + strlen($body)) . $form . $body;
}

function id3(int $minor, string $frames, int $flags = 0): string {
    $n = strlen($frames);
    $size = chr(($n >> 21) & 0x7f) . chr(($n >> 14) & 0x7f) . chr(($n >> 7) & 0x7f) . chr($n & 0x7f);
    return "ID3" . chr($minor) . "\x00" . chr($flags) . $size . $frames;
}

$mp3_frame = "\xff\xfb\x90\x00" . str_repeat("\x00", 413);
$fmt = pack('vvVVvv', 1, 2, 44100, 176400, 4, 16);

$samples = [
    'mp3 frame'   => $mp3_frame,
    'mp3 id3v2.3' => id3(3, str_repeat("\x00", 20)) . $mp3_frame,
    'mp3 id3v2.4' => id3(4, str_repeat("\x00", 20)) . $mp3_frame,
    'id3 only'    => id3(3, str_repeat("\x00", 20)),
    'mp2 frame'   => "\xff\xfd\x80\x00" . str_repeat("\x00", 100),
    'aac adts'    => "\xff\xf1\x50\x80" . str_repeat("\x00", 100),
    'mp4 isom'    => ftyp('isom', 'isomiso2avc1mp41') . box('mdat', 'x'),
    'mp4 mp42'    => ftyp('mp42', 'mp42isom'),
    'm4a'         => ftyp('M4A ', 'M4A mp42isom'),
    'm4v'         => ftyp('M4V ', 'M4V mp42isom'),
    'mov'         => ftyp('qt  ', 'qt  '),
    '3gp'         => ftyp('3gp4', '3gp4isom'),
    'mov moov'    => box('moov', box('mvhd', str_repeat("\x00", 100))),
    'ogg vorbis'  => "OggS\x00\x02" . str_repeat("\x00", 20) . "\x01vorbis" . str_repeat("\x00", 20),
    'ogg opus'    => "OggS\x00\x02" . str_repeat("\x00", 20) . "OpusHead" . str_repeat("\x00", 20),
    'ogg theora'  => "OggS\x00\x02" . str_repeat("\x00", 20) . "\x80theora" . str_repeat("\x00", 20),
    'ogg flac'    => "OggS\x00\x02" . str_repeat("\x00", 20) . "\x7fFLAC" . str_repeat("\x00", 20),
    'webm'        => "\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\xf7\x81\x01\x42\xf2\x81\x04\x42\xf3\x81\x08\x42\x82\x84webm\x42\x87\x81\x02",
    'mkv'         => "\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\xf7\x81\x01\x42\xf2\x81\x04\x42\xf3\x81\x08\x42\x82\x88matroska\x42\x87\x81\x02",
    'wav'         => riff('WAVE', 'fmt ' . pack('V', 16) . $fmt . 'data' . pack('V', 4) . "\x00\x00\x00\x00"),
    'avi'         => riff('AVI ', 'LIST' . pack('V', 4) . 'hdrl'),
    'flac'        => "fLaC\x00\x00\x00\x22" . str_repeat("\x00", 34),
    'midi'        => "MThd\x00\x00\x00\x06\x00\x01\x00\x02\x01\xe0",
    'aiff'        => "FORM\x00\x00\x00\x20AIFFCOMM",
    'pdf 1.4'     => "%PDF-1.4\n%\xe2\xe3\xcf\xd3\n1 0 obj\n<</Type/Catalog>>\nendobj\n",
    'pdf 1.7'     => "%PDF-1.7\n1 0 obj\n<</Type/Pages/Count 3>>\nendobj\n",
    'pdf bom'     => "\n%PDF-1.3\n",
    'woff'        => "wOFF\x00\x01\x00\x00" . pack('N', 100) . str_repeat("\x00", 30),
    'woff2'       => "wOF2\x00\x01\x00\x00" . pack('N', 100) . str_repeat("\x00", 40),
    'ttf'         => "\x00\x01\x00\x00" . pack('nnnn', 3, 0x20, 1, 0) . 'cmap' . str_repeat("\x00", 100),
    'otf'         => "OTTO" . pack('nnnn', 3, 0x20, 1, 0) . 'CFF ' . str_repeat("\x00", 100),
    'eot'         => str_repeat("\x00", 8) . str_repeat("\x00", 0x40) . "LP",
    'ps'          => "%!PS-Adobe-3.0\n%%Title: x\n",
    'sqlite'      => "SQLite format 3\x00" . pack('n', 4096) . str_repeat("\x00", 100),
    'elf64'       => "\x7fELF\x02\x01\x01\x00" . str_repeat("\x00", 8) . pack('vvV', 3, 62, 1) . str_repeat("\x00", 40),
    'exe'         => "MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xff\xff\x00\x00" . str_repeat("\x00", 44) . pack('V', 0x40) . str_repeat("\x00", 4) . "PE\x00\x00\x4c\x01" . str_repeat("\x00", 100),
    'java class'  => "\xca\xfe\xba\xbe\x00\x00\x00\x34" . str_repeat("\x00", 20),
    'wasm'        => "\x00asm\x01\x00\x00\x00",
    'random'      => "\x9c\x3f\x11\xa2\x55\x80\xe0\x01\x02\x03",
    'zeros'       => str_repeat("\x00", 64),
    'ones'        => str_repeat("\xff", 16),
];
foreach ($samples as $label => $data) {
    show($label, $data);
}
