<?php
function dump($d, $w, $h) {
    for ($y = 0; $y < $h; $y++) { for ($x = 0; $x < $w; $x++) echo imagecolorat($d, $x, $y) ? '#' : '.'; echo "\n"; }
    echo "--\n";
}
foreach ([[8, 6], [12, 10], [20, 20], [15, 7]] as [$w, $h]) {
    $d = imagecreatetruecolor(24, 24); imageellipse($d, 11, 11, $w, $h, 1); dump($d, 24, 24);
    $d = imagecreatetruecolor(24, 24); imagefilledellipse($d, 11, 11, $w, $h, 1); dump($d, 24, 24);
}
foreach ([[0, 360], [30, 200], [270, 90], [10, 10], [0, 90]] as [$s, $e]) {
    foreach ([IMG_ARC_PIE | IMG_ARC_NOFILL, IMG_ARC_PIE, IMG_ARC_CHORD, IMG_ARC_EDGED | IMG_ARC_NOFILL, IMG_ARC_CHORD | IMG_ARC_NOFILL] as $st) {
        $d = imagecreatetruecolor(24, 24); imagefilledarc($d, 11, 11, 18, 14, $s, $e, 1, $st); echo "arc $s $e $st\n"; dump($d, 24, 24);
    }
    $d = imagecreatetruecolor(24, 24); imagearc($d, 11, 11, 18, 14, $s, $e, 1); echo "imagearc $s $e\n"; dump($d, 24, 24);
}
