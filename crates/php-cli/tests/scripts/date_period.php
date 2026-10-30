<?php
$s = new DateTime('2026-01-01 10:00:00');
$i = new DateInterval('P1D');
$show = function ($p) { foreach ($p as $k => $d) echo $k, ' ', get_class($d), ' ', $d->format('Y-m-d H:i'), "\n"; echo "--\n"; };
$show(new DatePeriod($s, $i, 3));
$show(new DatePeriod($s, $i, 3, DatePeriod::EXCLUDE_START_DATE));
$show(new DatePeriod($s, $i, new DateTime('2026-01-04 10:00:00')));
$show(new DatePeriod($s, $i, new DateTime('2026-01-04 10:00:00'), DatePeriod::INCLUDE_END_DATE));
$show(new DatePeriod($s, $i, new DateTime('2026-01-04 10:00:00'), DatePeriod::EXCLUDE_START_DATE | DatePeriod::INCLUDE_END_DATE));
$show(new DatePeriod(new DateTimeImmutable('2026-01-31'), new DateInterval('P1M'), 2));
$p = new DatePeriod($s, $i, 2);
var_dump($p->getRecurrences(), $p->getEndDate(), get_class($p->getStartDate()), $p->getStartDate()->format('c'), $p->getDateInterval()->d);
var_dump($p->recurrences, $p->include_start_date, $p->include_end_date, $p->current);
$q = new DatePeriod($s, $i, new DateTime('2026-01-03'));
var_dump($q->getRecurrences(), $q->getEndDate()->format('Y-m-d'), $q->recurrences);
$x = new DatePeriod($s, $i, 2, DatePeriod::EXCLUDE_START_DATE);
var_dump($x->getRecurrences(), $x->recurrences, $x->include_start_date);
$s->modify('+1 year'); echo $p->getStartDate()->format('Y'), "\n";
var_dump($p instanceof IteratorAggregate, $p instanceof Traversable, iterator_count($p->getIterator()));
try { new DatePeriod($s, $i, 0); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
class MyPeriod extends DatePeriod {}
$m = new MyPeriod($s, $i, 1); echo get_class($m), ' ', iterator_count($m), "\n";
