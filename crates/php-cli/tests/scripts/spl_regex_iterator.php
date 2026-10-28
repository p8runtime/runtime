<?php
// RegexIterator's modes and flags, RecursiveRegexIterator, and the SPL
// iterators passing unknown method calls to their inner iterator.
$a = new ArrayIterator(['apple' => 'a1', 'banana' => 'b22', 'cherry' => 'c333', 'date' => 'x', 'n' => 42]);
$show = function ($it) { foreach ($it as $k => $v) echo json_encode($k), '=>', json_encode($v), ' '; echo "\n"; };
$show(new RegexIterator($a, '/\d+/'));
$show(new RegexIterator($a, '/(\w)(\d+)/', RegexIterator::GET_MATCH));
$show(new RegexIterator($a, '/\d/', RegexIterator::ALL_MATCHES));
$show(new RegexIterator($a, '/\d/', RegexIterator::SPLIT));
$r = new RegexIterator($a, '/\d/', RegexIterator::REPLACE);
$r->replacement = '#';
$show($r);
$show(new RegexIterator($a, '/an/', RegexIterator::MATCH, RegexIterator::USE_KEY));
$show(new RegexIterator($a, '/\d/', RegexIterator::MATCH, RegexIterator::INVERT_MATCH));
$k = new RegexIterator($a, '/a/', RegexIterator::REPLACE, RegexIterator::USE_KEY);
$k->replacement = 'A';
$show($k);
$show(new RegexIterator($a, '/(\d)/', RegexIterator::GET_MATCH, 0, PREG_OFFSET_CAPTURE));
$r = new RegexIterator($a, '/x/', RegexIterator::SPLIT);
var_dump($r->getMode(), $r->getFlags(), $r->getPregFlags(), $r->getRegex());
$r->setMode(RegexIterator::MATCH); $r->setFlags(RegexIterator::INVERT_MATCH); $r->setPregFlags(PREG_OFFSET_CAPTURE);
var_dump($r->getMode(), $r->getFlags(), $r->getPregFlags());
try { $r->setMode(9); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { new RegexIterator($a, '/x/', -1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { new RegexIterator($a, 'not a regex'); } catch (InvalidArgumentException $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
$nested = new RecursiveArrayIterator(['x1', ['y2', 'zz', ['w3']], 'q']);
foreach (new RecursiveIteratorIterator(new RecursiveRegexIterator($nested, '/\d/')) as $k => $v) echo "$k=>", json_encode($v), ' ';
echo "\n";
$rri = new RecursiveRegexIterator($nested, '/^[a-z]\d$/', RegexIterator::GET_MATCH);
foreach (new RecursiveIteratorIterator($rri) as $k => $v) echo "$k=>", json_encode($v), ' ';
echo "\n";
// Forwarding: getArrayCopy() is the inner ArrayIterator's.
$f = new IteratorIterator(new ArrayIterator([3, 1, 2]));
var_dump($f->count(), $f->getArrayCopy());
$rit = new RecursiveIteratorIterator(new RecursiveArrayIterator([1, [2, 3]]));
foreach ($rit as $v) { if ($v === 2) var_dump($rit->getArrayCopy()); }
try { $f->nope(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(RecursiveRegexIterator::ALL_MATCHES, RecursiveDirectoryIterator::SKIP_DOTS, GlobIterator::CURRENT_AS_PATHNAME);
