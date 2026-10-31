<?php
// Collator, MessageFormatter, Normalizer, grapheme_*, idn_*, IntlChar.
$c = new Collator('en_US');
var_dump($c->compare('a', 'B'), $c->compare('a', 'A'), $c->compare('résumé', 'resume'), $c->compare('abc', 'abc'), $c->compare("\xff", 'a'), $c->getErrorMessage());
$a = ['banana', 'Apple', 'cherry', 'apple', '10', '9', 'Äpfel', 'zebra', 'éclair'];
$b = $a;
var_dump($c->sort($b));
echo implode(',', $b), "\n";
$b = $a;
$c->sort($b, Collator::SORT_STRING);
echo implode(',', $b), "\n";
$b = $a;
collator_sort($c, $b, Collator::SORT_NUMERIC);
echo implode(',', $b), "\n";
$b = ['x' => 'b', 'y' => 'a', 'z' => 'C'];
$c->asort($b);
var_dump($b);
for ($i = 0; $i <= 7; $i++) {
    echo $i, '=', $c->getAttribute($i), ' ';
}
echo "\n", $c->getStrength(), "\n";
$c->setStrength(Collator::PRIMARY);
var_dump($c->compare('a', 'A'), $c->compare('a', 'á'));
$c->setAttribute(Collator::NUMERIC_COLLATION, Collator::ON);
$b = ['a10', 'a9', 'a100'];
$c->sort($b);
echo implode(',', $b), "\n";
$s = new Collator('sv_SE');
$b = ['ö', 'z', 'a', 'å', 'ä'];
$s->sort($b);
echo implode(',', $b), "\n";
var_dump($c->setAttribute(99, 1), $c->getErrorMessage(), $s->getLocale(Locale::VALID_LOCALE));
$u = new Collator('en');
$u->setAttribute(Collator::CASE_FIRST, Collator::UPPER_FIRST);
$b = ['a', 'A', 'b', 'B'];
$u->sort($b);
echo implode(',', $b), "\n";

$m = new MessageFormatter('en_US', '{0,number} monkeys on {1,number} trees make {2,number} monkeys per tree');
var_dump($m->format([4560, 123, 4560 / 123]), $m->getPattern(), $m->getLocale());
var_dump(MessageFormatter::formatMessage('de', '{0,number} Affen über {1,number} Bäume um {2,number} Affen pro Baum', [4560, 123, 4560 / 123]));
$p = '{n, plural, =0{no files} one{# file} other{# files}} in {dir}';
foreach ([0, 1, 2, 1234] as $n) {
    var_dump(msgfmt_format_message('en', $p, ['n' => $n, 'dir' => 'tmp']));
}
var_dump(MessageFormatter::formatMessage('en', '{g, select, female{She} male{He} other{They}} replied', ['g' => 'female']));
var_dump(MessageFormatter::formatMessage('en', '{n, selectordinal, one{#st} two{#nd} few{#rd} other{#th}}', ['n' => 23]));
var_dump(MessageFormatter::formatMessage('ru', '{n, plural, one{# файл} few{# файла} many{# файлов} other{# файла}}', ['n' => 5]));
var_dump(MessageFormatter::formatMessage('en', "It''s '{'quoted'}' and {0}", ['x']));
var_dump(MessageFormatter::formatMessage('en', '{0, number, integer} / {0, number, percent} / {1}', [0.756, 12345]));
var_dump(MessageFormatter::formatMessage('en', 'missing {1}', [0 => 'x']));
var_dump(MessageFormatter::formatMessage('en', '{name} is {age}', ['name' => 'Ann', 'age' => 1234]));
var_dump(MessageFormatter::formatMessage('en', '{0, date, yyyy-MM-dd}', [86400 * 365]));
var_dump(MessageFormatter::formatMessage('en', '{0, choice, 0#none|1#one|1<many}', [5]));
var_dump(msgfmt_create('en', '{0, plural, one{a}'), intl_get_error_code(), intl_get_error_message());
var_dump(msgfmt_create('en', '{0,whatever} x'), intl_get_error_message());
var_dump(MessageFormatter::parseMessage('en_US', '{0,number} monkeys on {1,number} trees', '4,560 monkeys on 123 trees'));

var_dump(Normalizer::normalize("e\u{0301}", Normalizer::FORM_C) === "\u{00E9}", bin2hex(Normalizer::normalize("\u{00E9}", Normalizer::FORM_D)), Normalizer::normalize("\u{FB01}", Normalizer::FORM_KC), Normalizer::normalize("ＡＢＣ", Normalizer::FORM_KC_CF));
var_dump(Normalizer::isNormalized("e\u{0301}"), Normalizer::isNormalized("e\u{0301}", Normalizer::FORM_D), normalizer_normalize("\xff"), intl_get_error_message());
var_dump(bin2hex(Normalizer::getRawDecomposition("\u{00E9}")), Normalizer::getRawDecomposition('a'));
try {
    Normalizer::normalize('x', 99);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}

$s = "e\u{0301}a\u{1F469}\u{200D}\u{1F4BB}b";
var_dump(grapheme_strlen($s), grapheme_substr($s, 1, 2), grapheme_substr($s, -1), grapheme_strpos($s, 'b'), grapheme_strpos($s, "\u{1F469}"), grapheme_stripos('ABCé', 'É'), grapheme_strrpos('abcabc', 'b'), grapheme_strstr($s, 'a'), grapheme_stristr('XAb', 'ab', true), grapheme_str_split($s, 2));
var_dump(grapheme_extract($s, 2, GRAPHEME_EXTR_COUNT, 0, $next), $next, grapheme_extract('abcdef', 3, GRAPHEME_EXTR_MAXBYTES, 1), grapheme_strlen("\xff"));

var_dump(idn_to_ascii('bücher.de'), idn_to_utf8('xn--bcher-kva.de'), idn_to_ascii('faß.de'), idn_to_ascii('faß.de', 0), idn_to_ascii('-a.de', IDNA_DEFAULT, INTL_IDNA_VARIANT_UTS46, $info), $info);

var_dump(IntlChar::ord('A'), IntlChar::chr(0x1F600), IntlChar::charType('A') === IntlChar::CHAR_CATEGORY_UPPERCASE_LETTER, IntlChar::isalpha('é'), IntlChar::isdigit('٣'), IntlChar::charDigitValue('٣'), IntlChar::toupper('ß'), IntlChar::tolower(0x41), IntlChar::totitle('ǆ'), IntlChar::isupper('A'), IntlChar::ispunct('!'), IntlChar::isspace("\t"), IntlChar::isWhitespace("\u{00A0}"), IntlChar::charMirror('('), IntlChar::getBidiPairedBracket(']'), IntlChar::charDirection('a'), IntlChar::getCombiningClass("\u{0301}"), IntlChar::hasBinaryProperty('a', IntlChar::PROPERTY_ALPHABETIC), IntlChar::getIntPropertyValue('a', IntlChar::PROPERTY_SCRIPT), IntlChar::getBlockCode('a') === IntlChar::BLOCK_CODE_BASIC_LATIN, IntlChar::digit('f', 16), IntlChar::digit('z'), IntlChar::forDigit(11, 16), IntlChar::foldCase('A'), IntlChar::ord('ab'), intl_get_error_message());

// The Unicode version the properties are of, as IntlChar::UNICODE_VERSION says.
var_dump(IntlChar::getUnicodeVersion(), implode('.', array_slice(IntlChar::getUnicodeVersion(), 0, 2)) === IntlChar::UNICODE_VERSION);
