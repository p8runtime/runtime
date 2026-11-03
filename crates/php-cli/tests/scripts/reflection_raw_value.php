<?php
class P {
    public int $n = 1;
    public string $h { set(string $v) { $this->h = strtoupper($v); } get => $this->h . '!'; }
    public static $s = 0;
}
$p = new P;
$r = new ReflectionProperty(P::class, 'n');
$r->setRawValue($p, 5);
var_dump($r->getRawValue($p), $p->n);
$h = new ReflectionProperty(P::class, 'h');
$h->setRawValue($p, 'abc');
var_dump($h->getRawValue($p), $p->h);
$p->h = 'xyz';
var_dump($h->getRawValue($p), $p->h);
try { (new ReflectionProperty(P::class, 's'))->setRawValue($p, 1); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
