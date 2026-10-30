<?php
// NumberFormatter: CLDR's decimal, percent and currency patterns and
// symbols, ICU's pattern syntax, attributes, rounding, parsing.
foreach (['en_US', 'de_DE', 'fr_FR', 'es_ES', 'en_IN', 'ja_JP', 'pt_BR', 'ru_RU', 'nl_NL', 'sv_SE', 'zh_CN', 'th_TH', 'en'] as $l) {
    $d = new NumberFormatter($l, NumberFormatter::DECIMAL);
    $c = new NumberFormatter($l, NumberFormatter::CURRENCY);
    $p = new NumberFormatter($l, NumberFormatter::PERCENT);
    echo "$l | ", $d->format(-1234567.891), ' | ', $d->format(1234), ' | ', $d->getPattern(), ' | ', $c->format(-1234.5), ' | ', $c->getPattern(), ' | ', $c->getTextAttribute(NumberFormatter::CURRENCY_CODE), ' | ', $c->formatCurrency(1234.5, 'USD'), ' | ', $p->format(-0.256), ' | ', $p->getPattern(), ' | ', $d->getSymbol(NumberFormatter::DECIMAL_SEPARATOR_SYMBOL), $d->getSymbol(NumberFormatter::GROUPING_SEPARATOR_SYMBOL), $d->getSymbol(NumberFormatter::PERCENT_SYMBOL), ' | ', $d->getLocale(), "\n";
}

$f = new NumberFormatter('en_US', NumberFormatter::DECIMAL);
foreach ([0, 1, -1, 0.5, 1.5, 2.5, 1e20, 1.23456789e-7, NAN, INF, -INF, PHP_INT_MAX, -0.0, 0.1 + 0.2, 1.005, 2.675, 123456789.12345678] as $v) {
    echo var_export($v, true), ' => ', $f->format($v), "\n";
}
var_dump($f->format(1234.5678, NumberFormatter::TYPE_INT32), $f->format(1234.5678, NumberFormatter::TYPE_INT64), $f->format(1234.5678, NumberFormatter::TYPE_DOUBLE), numfmt_format($f, 42));

foreach ([[NumberFormatter::FRACTION_DIGITS, 2], [NumberFormatter::MIN_FRACTION_DIGITS, 4], [NumberFormatter::GROUPING_USED, 0], [NumberFormatter::ROUNDING_MODE, NumberFormatter::ROUND_DOWN], [NumberFormatter::ROUNDING_MODE, NumberFormatter::ROUND_HALFUP], [NumberFormatter::MULTIPLIER, 10], [NumberFormatter::MIN_INTEGER_DIGITS, 8], [NumberFormatter::FORMAT_WIDTH, 15], [NumberFormatter::DECIMAL_ALWAYS_SHOWN, 1], [NumberFormatter::GROUPING_SIZE, 2], [NumberFormatter::ROUNDING_INCREMENT, 0.05], [NumberFormatter::MAX_INTEGER_DIGITS, 2]] as [$k, $v]) {
    $f = new NumberFormatter('en_US', NumberFormatter::DECIMAL);
    $ok = $f->setAttribute($k, $v);
    echo "$k=$v: ", var_export($ok, true), ' ', $f->format(1234.5678), ' ', $f->format(-0.001), ' ', $f->getAttribute($k), "\n";
}
$f = new NumberFormatter('en_US', NumberFormatter::DECIMAL);
for ($i = 0; $i <= 19; $i++) {
    echo $i, '=', var_export($f->getAttribute($i), true), ' ';
}
echo "\n";
for ($i = 0; $i <= 5; $i++) {
    echo $i, '=', var_export($f->getTextAttribute($i), true), ' ';
}
echo "\n";

foreach (['#,##0.00;(#,##0.00)', '0.00E0', '##0.###E0', '00.###E0', "'#'#", '#,##0.00 ¤¤', '#%', '#,##0.0#', '0000', '#,##,##0', '¤#,##0.00;¤-#,##0.00'] as $pat) {
    $f = new NumberFormatter('en_US', NumberFormatter::PATTERN_DECIMAL, $pat);
    echo $pat, ' => ', $f->format(-12345.678), ' ', $f->format(0.01234), ' ', $f->getPattern(), "\n";
}

$f = new NumberFormatter('en_US', NumberFormatter::DECIMAL);
var_dump($f->parse('1,234.56'), $f->parse('1,234.56', NumberFormatter::TYPE_INT64), $f->parse('abc'), $f->getErrorCode(), $f->getErrorMessage());
$pos = 0;
var_dump($f->parse('12abc', NumberFormatter::TYPE_DOUBLE, $pos), $pos, $f->parse('-1e3'), $f->parse('.5'), $f->parse('1,2,3,4.5'));
$pos = 0;
var_dump(numfmt_parse($f, '7 apples', NumberFormatter::TYPE_INT32, $pos), $pos);
$c = new NumberFormatter('en_US', NumberFormatter::CURRENCY);
var_dump($c->parseCurrency('$1,234.50', $cur), $cur, $c->parseCurrency('EUR1,234.50', $cur), $cur);
$p = new NumberFormatter('en_US', NumberFormatter::PERCENT);
var_dump($p->parse('50%'), $p->parse('50'));
$g = new NumberFormatter('de_DE', NumberFormatter::DECIMAL);
var_dump($g->parse('1.234,5'), $g->parse('1234.5'));

$c = new NumberFormatter('en_US', NumberFormatter::CURRENCY);
$c->setAttribute(NumberFormatter::FRACTION_DIGITS, 3);
var_dump($c->formatCurrency(1.5, 'USD'), $c->formatCurrency(1.5, 'JPY'), $c->getPattern());
$c = new NumberFormatter('en_US', NumberFormatter::CURRENCY);
var_dump($c->formatCurrency(5, 'XYZ'), $c->formatCurrency(5, 'usd'), $c->formatCurrency(5, 'EURO'));
$c = new NumberFormatter('ja_JP', NumberFormatter::CURRENCY);
var_dump($c->getAttribute(NumberFormatter::MAX_FRACTION_DIGITS), $c->format(1234.5));
$c = new NumberFormatter('en_US@currency=EUR', NumberFormatter::CURRENCY);
var_dump($c->format(1234.5));
$t = new NumberFormatter('th_TH@numbers=thai', NumberFormatter::DECIMAL);
var_dump($t->format(1234.5));

$f = new NumberFormatter('de_DE', NumberFormatter::DECIMAL);
var_dump($f->setPattern('#,##0.00 ¤'), $f->format(5), $f->getPattern());
var_dump($f->setTextAttribute(NumberFormatter::POSITIVE_PREFIX, '+'), $f->format(5.5), $f->getPattern());
var_dump($f->setPattern('0.0     .#.#.#'), $f->getErrorCode(), $f->getErrorMessage());
$e = new NumberFormatter('en_US', NumberFormatter::DECIMAL);
var_dump($e->setTextAttribute(NumberFormatter::NEGATIVE_PREFIX, '('), $e->setTextAttribute(NumberFormatter::NEGATIVE_SUFFIX, ')'), $e->format(-5), $e->getPattern());
var_dump($e->setAttribute(99, 1), $e->getErrorMessage(), $e->getSymbol(99), intl_get_error_message());
$clone = clone $e;
var_dump($clone->format(-7));

try {
    new NumberFormatter('en_US', 99);
} catch (IntlException $ex) {
    echo get_class($ex), ': ', $ex->getMessage(), "\n";
}
var_dump(numfmt_create('en_US', 99), intl_get_error_message());
try {
    new NumberFormatter('xx', NumberFormatter::DECIMAL);
} catch (ValueError $ex) {
    echo get_class($ex), ': ', $ex->getMessage(), "\n";
}
try {
    $e->format(1, 4); // NumberFormatter::TYPE_CURRENCY
} catch (ValueError $ex) {
    echo get_class($ex), ': ', $ex->getMessage(), "\n";
}
