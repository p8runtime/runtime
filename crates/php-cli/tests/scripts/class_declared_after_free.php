<?php
// Each eval'd program is freed after it runs; a later class must still be declared.
function declare_in(string $code) { eval($code); }
$missing = [];
for ($i = 0; $i < 200; $i++) {
    declare_in("class P$i {}");
    declare_in("class C$i extends P$i {}");
    if (!class_exists("C$i", false)) $missing[] = "C$i";
}
echo count($missing), " missing\n";
