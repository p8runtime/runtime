<?php
$i = 0;
start:
$i++;
if ($i < 3) goto start;
echo "i=$i\n";
goto skip;
echo "never\n";
skip:
echo "skipped\n";
function f($n) {
    $out = [];
    loop:
    if ($n <= 0) goto done;
    $out[] = $n--;
    goto loop;
    done:
    return $out;
}
print_r(f(3));
for ($a = 0; $a < 10; $a++) {
    foreach ([1, 2, 3] as $b) {
        if ($a * $b == 4) goto out;
    }
}
out:
echo "a=$a b=$b\n";
while (true) {
    inner:
    $a--;
    if ($a > 0) goto inner;
    break;
}
echo "a=$a\n";
