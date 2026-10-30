<?php
// array_unique under each flag, as php_array.c does it (a set of strings for
// SORT_STRING; sorted runs for the others): the first of equal entries stays.
$sets = [
  [3, '3', 'a', 'A', 1.0, 1, true, null, '', 0, '0', 'abc', '1e1', 10, ' 1', '1 '],
  ['x' => 'b', 'y' => 'a', 'z' => 'b', 5 => 'a', 6 => 'c'],
  [[1, 2], [1, 2], [2], 'Array'],
  [0.1 + 0.2, 0.3, '0.3', 0.30000000000000004],
  ['10', '9', '9a', '1e1', 'abc', 'ABC', 'abc '],
];
foreach ($sets as $i => $s) {
    foreach ([SORT_STRING, SORT_REGULAR, SORT_NUMERIC, SORT_LOCALE_STRING] as $f) {
        if ($i === 2 && $f !== SORT_REGULAR) continue; // arrays to string warn with the file path
        echo "set $i flags $f: ", json_encode(@array_unique($s, $f)), "\n";
    }
}
echo json_encode(array_unique([4, "4", "3", 4, 3, "3"])), "\n";
