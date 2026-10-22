<?php
function boom($x) { if ($x == 2) throw new RuntimeException("boom $x"); return $x * 2; }
try { array_map('boom', [1, 2, 3]); } catch (RuntimeException $e) { echo "caught ", $e->getMessage(), "\n"; }
$arr = [3, 1, 2]; try { usort($arr, function ($a, $b) { throw new LogicException("cmp"); }); } catch (LogicException $e) { echo "caught ", $e->getMessage(), "\n"; }
try { call_user_func('boom', 2); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
try { preg_replace_callback('/a/', function ($m) { throw new Exception("re"); }, 'abc'); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
echo "done\n";
