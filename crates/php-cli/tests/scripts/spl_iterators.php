<?php
$a = new ArrayIterator(['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4, 'e' => 5]);
foreach (new IteratorIterator($a) as $k => $v) echo "$k=$v ";
echo "\n";
class Odd extends FilterIterator { public function accept(): bool { return $this->current() % 2 == 1; } }
foreach (new Odd($a) as $k => $v) echo "$k=$v ";
echo "\n";
foreach (new CallbackFilterIterator($a, fn($v, $k) => $k > 'b') as $k => $v) echo "$k=$v ";
echo "\n";
foreach (new LimitIterator($a, 1, 3) as $k => $v) echo "$k=$v ";
echo "\n";
$li = new LimitIterator(new ArrayIterator(range(10, 20)), 2, 5); $li->rewind(); $li->seek(4); var_dump($li->current(), $li->getPosition());
try { $li->seek(0); } catch (OutOfBoundsException $e) { echo $e->getMessage(), "\n"; }
$c = new CachingIterator(new ArrayIterator(['x', 'y', 'z']));
foreach ($c as $v) { echo $v, $c->hasNext() ? ', ' : "\n"; }
$fc = new CachingIterator(new ArrayIterator(['p' => 1, 'q' => 2]), CachingIterator::FULL_CACHE);
foreach ($fc as $v) {} var_dump($fc->getCache(), count($fc), $fc['q']);
$nr = new NoRewindIterator(new ArrayIterator([1, 2, 3])); $nr->next();
foreach ($nr as $v) echo $v; echo "\n";
$inf = new InfiniteIterator(new ArrayIterator([1, 2])); $n = 0;
foreach ($inf as $v) { echo $v; if (++$n >= 5) break; } echo "\n";
$ap = new AppendIterator; $ap->append(new ArrayIterator([1, 2])); $ap->append(new ArrayIterator([])); $ap->append(new ArrayIterator([3]));
foreach ($ap as $k => $v) echo "$k=$v(", $ap->getIteratorIndex(), ") "; echo "\n";
foreach (new EmptyIterator as $v) echo "never";
try { (new EmptyIterator)->current(); } catch (BadMethodCallException $e) { echo $e->getMessage(), "\n"; }
$tree = ['a' => 1, 'b' => ['c' => 2, 'd' => ['e' => 3]], 'f' => 4];
foreach (new RecursiveIteratorIterator(new RecursiveArrayIterator($tree)) as $k => $v) echo "$k=$v ";
echo "\n";
$r = new RecursiveIteratorIterator(new RecursiveArrayIterator($tree), RecursiveIteratorIterator::SELF_FIRST);
foreach ($r as $k => $v) echo str_repeat(' ', $r->getDepth()), $k, is_array($v) ? '' : "=$v", "\n";
$r = new RecursiveIteratorIterator(new RecursiveArrayIterator($tree), RecursiveIteratorIterator::CHILD_FIRST);
foreach ($r as $k => $v) echo $k, ' '; echo "\n";
$r = new RecursiveIteratorIterator(new RecursiveArrayIterator($tree), RecursiveIteratorIterator::SELF_FIRST); $r->setMaxDepth(0);
foreach ($r as $k => $v) echo $k, ' '; echo "\n"; var_dump($r->getMaxDepth());
class Tracer extends RecursiveIteratorIterator {
    function beginIteration(): void { echo "[begin]"; }
    function endIteration(): void { echo "[end]"; }
    function beginChildren(): void { echo "<"; }
    function endChildren(): void { echo ">"; }
}
foreach (new Tracer(new RecursiveArrayIterator([1, [2, [3]], 4])) as $v) echo $v; echo "\n";
foreach (new ParentIterator(new RecursiveArrayIterator($tree)) as $k => $v) echo $k, ' '; echo "\n";
$rcf = new RecursiveCallbackFilterIterator(new RecursiveArrayIterator($tree), fn($v, $k, $it) => $it->hasChildren() || $v > 1);
foreach (new RecursiveIteratorIterator($rcf) as $k => $v) echo "$k=$v "; echo "\n";
function gen() { yield 'g1' => 1; yield 'g2' => 2; }
foreach (new IteratorIterator(gen()) as $k => $v) echo "$k=$v "; echo "\n";
var_dump(iterator_to_array(new LimitIterator(new ArrayIterator([5, 6, 7, 8]), 1, 2)));
print_r(new IteratorIterator(new ArrayIterator([1]))); echo "\n";
