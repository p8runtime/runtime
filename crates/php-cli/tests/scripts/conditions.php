<?php
// Conditions with &&, ||, !, and/or: what runs, in what order, and the result.
function t($name, $v) { echo $name, " "; return $v; }
function check($label, $c) { echo $label, ": ", $c ? "yes" : "no", "\n"; }
if (t("a", true) && t("b", false)) { echo "then\n"; } else { echo "else\n"; }
if (t("a", false) && t("b", true)) { echo "then\n"; } else { echo "else\n"; }
if (t("a", false) || t("b", true)) { echo "then\n"; } else { echo "else\n"; }
if (t("a", true) || t("b", false)) { echo "then\n"; } else { echo "else\n"; }
if (!(t("a", true) && t("b", true))) { echo "then\n"; } else { echo "else\n"; }
if (!t("a", 0) || t("b", 1)) { echo "then\n"; } else { echo "else\n"; }
if ((t("a", 1) && t("b", 0)) || (t("c", "0") || t("d", [1]))) { echo "then\n"; } else { echo "else\n"; }
if (t("a", "") and t("b", 1) or t("c", 2)) { echo "then\n"; } else { echo "else\n"; }
foreach ([[1, 2], [3, 2], [0, 0], [-1, 5]] as $p) {
    $x = $p[0];
    $y = $p[1];
    if ($x > 0 && $y > 0 && $x !== $y) { echo "$x,$y both positive, differ\n"; }
    elseif ($x == 0 || $y == 0) { echo "$x,$y has a zero\n"; }
    elseif (!($x < $y)) { echo "$x,$y x not below y\n"; }
    else { echo "$x,$y other\n"; }
}
$i = 0; $n = 0;
while ($i < 10 && $n < 3) { if ($i % 2 == 0 || $i == 7) { $n++; } $i++; }
echo "$i $n\n";
for ($i = 0; !($i >= 5) && $i != 3; $i++) { echo $i; }
echo "\n";
$k = 5;
do { $k--; } while ($k > 0 && !($k == 2));
echo $k, "\n";
echo ($k > 1 && $k < 3) ? "in\n" : "out\n";
echo (!$k || $k > 100) ? "a\n" : "b\n";
check("null", null && true);
check("str", "a" || false);
check("arr", [] || [0]);
check("not", !"0");
