<?php
// Regenerate: docker run --rm -i p8-php-apps:50ebaed2745d php < gen/uloc_tables.php > src/uloc_tables.rs (PHP 8.4 with ICU 76.1)
$a = range('a','z'); $langs = []; $regs = [];
foreach ($a as $x) foreach ($a as $y) foreach ($a as $z) {
  $c = Locale::canonicalize("$x$y$z"); if ($c !== "$x$y$z" && $c !== '') $langs[] = "    (\"$x$y$z\", \"$c\"),";
  $r = strtoupper("$x$y$z"); $c = Locale::canonicalize("en_$r"); if ($c !== "en_$r") $regs[] = "    (\"$r\", \"" . substr($c, 3) . "\"),";
}
echo "//! ICU ", INTL_ICU_VERSION, "'s ISO 639-2 to 639-1 and ISO 3166 alpha-3 to alpha-2\n//! mappings (what uloc_getLanguage / uloc_getCountry apply), from gen/uloc_tables.php.\n\n";
echo "pub static LANG3: &[(&str, &str)] = &[\n", implode("\n", $langs), "\n];\n\n";
echo "pub static REGION3: &[(&str, &str)] = &[\n", implode("\n", $regs), "\n];\n\n";
$l = ResourceBundle::getLocales('');
echo "/// ICU's available locales (ures_openAvailableLocales), for acceptFromHttp.\n";
echo "pub static AVAILABLE: &[&str] = &[\n";
foreach (array_chunk($l, 10) as $chunk) echo "    ", implode(", ", array_map(fn($x) => "\"$x\"", $chunk)), ",\n";
echo "];\n";
