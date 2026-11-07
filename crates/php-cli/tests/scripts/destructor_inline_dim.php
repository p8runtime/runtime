<?php
// The last holder is an array element replaced by a string-key write,
// and by an int-key write.
class D { function __destruct() { echo "D\n"; } }
function f() {
    $a = ['k' => new D];
    $a['k'] = 1;
    echo "after string key\n";
    $b = [new D];
    $b[0] = 2;
    echo "after int key\n";
}
f();
