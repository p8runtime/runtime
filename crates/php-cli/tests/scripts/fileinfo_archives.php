<?php
// Archives and the zip-based document formats; the zips are written here.
$flags = [FILEINFO_NONE, FILEINFO_MIME_TYPE, FILEINFO_MIME, FILEINFO_MIME_ENCODING, FILEINFO_EXTENSION];

function show(string $label, string $data): void {
    global $flags;
    $row = [];
    foreach ($flags as $f) {
        $row[] = (new finfo($f))->buffer($data);
    }
    echo str_pad($label, 14), ' ', implode(' | ', $row), "\n";
}

/** A zip of stored entries (name => data). */
function zip(array $entries, string $comment = ''): string {
    $out = '';
    $central = '';
    foreach ($entries as $name => $data) {
        $crc = hexdec(hash('crc32b', $data));
        $offset = strlen($out);
        $local = pack('VvvvvvVVVvv', 0x04034b50, 20, 0, 0, 0, 0x21, $crc, strlen($data), strlen($data), strlen($name), 0) . $name;
        $out .= $local . $data;
        $central .= pack('VvvvvvvVVVvvvvvVV', 0x02014b50, 20, 20, 0, 0, 0, 0x21, $crc, strlen($data), strlen($data), strlen($name), 0, 0, 0, 0, 0, $offset) . $name;
    }
    $end = pack('VvvvvVVv', 0x06054b50, 0, 0, count($entries), count($entries), strlen($central), strlen($out), strlen($comment)) . $comment;
    return $out . $central . $end;
}

function tar_header(string $name, int $size, string $magic = "ustar\x0000"): string {
    $h = str_pad($name, 100, "\0") . sprintf('%07o', 0644) . "\0" . sprintf('%07o', 0) . "\0" . sprintf('%07o', 0) . "\0"
        . sprintf('%011o', $size) . "\0" . sprintf('%011o', 0) . "\0" . '        ' . '0' . str_repeat("\0", 100) . $magic;
    $h = str_pad($h, 512, "\0");
    $sum = array_sum(array_map('ord', str_split($h)));
    return substr($h, 0, 148) . sprintf('%06o', $sum) . "\0 " . substr($h, 156);
}

$ct = '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>';
$docx = zip(['[Content_Types].xml' => $ct, '_rels/.rels' => '<r/>', 'word/document.xml' => '<w/>']);
$xlsx = zip(['[Content_Types].xml' => $ct, '_rels/.rels' => '<r/>', 'xl/workbook.xml' => '<w/>']);
$pptx = zip(['[Content_Types].xml' => $ct, '_rels/.rels' => '<r/>', 'ppt/presentation.xml' => '<w/>']);
$odt = zip(['mimetype' => 'application/vnd.oasis.opendocument.text', 'content.xml' => '<c/>']);
$ods = zip(['mimetype' => 'application/vnd.oasis.opendocument.spreadsheet', 'content.xml' => '<c/>']);
$epub = zip(['mimetype' => 'application/epub+zip', 'META-INF/container.xml' => '<c/>']);
$jar = zip(['META-INF/MANIFEST.MF' => "Manifest-Version: 1.0\n", 'A.class' => "\xca\xfe\xba\xbe"]);
$apk = zip(['AndroidManifest.xml' => 'x', 'classes.dex' => 'dex']);
$plain = zip(['a.txt' => 'hello', 'dir/b.txt' => 'world']);
$empty = zip([]);

$gzip = gzencode('hello hello hello');
$samples = [
    'zip'         => $plain,
    'zip empty'   => $empty,
    'zip comment' => zip(['a.txt' => 'x'], 'a comment'),
    'docx'        => $docx,
    'xlsx'        => $xlsx,
    'pptx'        => $pptx,
    'odt'         => $odt,
    'ods'         => $ods,
    'epub'        => $epub,
    'jar'         => $jar,
    'apk'         => $apk,
    'zip head'    => "PK\x03\x04\x14\x00\x00\x00\x08\x00",
    'gzip'        => $gzip,
    'gzip named'  => "\x1f\x8b\x08\x08\x00\x00\x00\x00\x00\x03file.txt\x00" . substr($gzip, 10),
    'bzip2'       => "BZh91AY&SY\x00\x00\x00\x00",
    'xz'          => "\xfd7zXZ\x00\x00\x04\xe6\xd6\xb4\x46",
    'zstd'        => "\x28\xb5\x2f\xfd\x24\x00",
    'lz4'         => "\x04\x22\x4d\x18\x64\x40\xa7",
    '7z'          => "7z\xbc\xaf\x27\x1c\x00\x04" . str_repeat("\x00", 24),
    'rar4'        => "Rar!\x1a\x07\x00" . str_repeat("\x00", 20),
    'rar5'        => "Rar!\x1a\x07\x01\x00" . str_repeat("\x00", 20),
    'tar'         => tar_header('a.txt', 5) . str_pad('hello', 512, "\0") . str_repeat("\0", 1024),
    'tar v7'      => tar_header('a.txt', 5, str_repeat("\0", 8)) . str_pad('hello', 512, "\0"),
    'tar gnu'     => tar_header('a.txt', 5, "ustar  \0") . str_pad('hello', 512, "\0"),
    'cpio'        => "070701" . str_repeat('0', 104) . "TRAILER!!!\0",
    'ar'          => "!<arch>\n" . str_pad('a.o/', 16) . str_repeat(' ', 32) . "`\n",
    'deb'         => "!<arch>\ndebian-binary   " . str_repeat(' ', 32) . "`\n2.0\n",
    'rpm'         => "\xed\xab\xee\xdb\x03\x00\x00\x00",
    'dmg koly'    => str_repeat("\x00", 512) . 'koly',
    'iso'         => str_repeat("\x00", 0x8001) . 'CD001' . str_repeat("\x00", 100),
    'ole2'        => "\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1" . str_repeat("\x00", 24),
];
foreach ($samples as $label => $data) {
    show($label, $data);
}
