<?php
// Writing a string offset: the first byte at the offset (padded with spaces), and PHP's errors and warnings.
$cases = [
  fn() => (function () { $s = 'abc'; $r = ($s[1] = 'XY'); return [$s, $r]; })(),
  fn() => (function () { $s = 'abc'; $s[5] = 'Y'; return $s; })(),
  fn() => (function () { $s = 'abc'; $s[-1] = 'Z'; return $s; })(),
  fn() => (function () { $s = 'abc'; $s[-5] = 'Z'; return $s; })(),
  fn() => (function () { $s = 'abc'; $s[0] = ''; return $s; })(),
  fn() => (function () { $s = 'abc'; $s[] = 'd'; return $s; })(),
  fn() => (function () { $s = 'abc'; $s[0][0] = 'd'; return $s; })(),
  fn() => (function () { $s = 'abc'; $s[0] .= 'd'; return $s; })(),
  fn() => (function () { $s = 'abc'; $s['x'] = 'd'; return $s; })(),
  fn() => (function () { $s = 'abc'; $s['1'] = 'd'; return $s; })(),
  fn() => (function () { $s = 'abc'; $s[' 1'] = 'd'; return $s; })(),
  fn() => (function () { $s = 'abc'; $s['1x'] = 'd'; return $s; })(),
  fn() => (function () { $s = 'abc'; $s[1.5] = 'd'; return $s; })(),
  fn() => (function () { $s = 'abc'; $s[true] = 'd'; return $s; })(),
  fn() => (function () { $s = 'abc'; $s[1] = 7; return $s; })(),
  fn() => (function () { $s = ''; $s[2] = 'q'; return $s; })(),
  fn() => (function () { $a = ['k' => 'xyz']; $a['k'][1] = 'Q'; return $a; })(),
  fn() => (function () { $o = new stdClass; $o->p = 'xyz'; $o->p[0] = 'Q'; return $o->p; })(),
  fn() => (function () { $s = 'abc'; $s[0]++; return $s; })(),
];
foreach ($cases as $i => $c) {
    try { echo $i, ': ', json_encode($c()), "\n"; } catch (Throwable $e) { echo $i, ': ', get_class($e), ': ', $e->getMessage(), "\n"; }
}
