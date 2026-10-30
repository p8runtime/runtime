<?php
// Regenerate: docker run --rm -i p8-php-apps:50ebaed2745d php < gen/currency_tables.php > src/currency_tables.rs (PHP 8.4 with ICU 76.1)
$regions = [];
foreach (ResourceBundle::getLocales('') as $l) { $r = Locale::getRegion($l); if ($r !== '') $regions[$r] = 1; }
foreach (range('A','Z') as $a) foreach (range('A','Z') as $b) $regions["$a$b"] = 1;
ksort($regions);
echo "//! CLDR's currency in use per region (supplemental currencyData, as ICU ", INTL_ICU_VERSION, "\n//! resolves it for a locale with no currency keyword), from gen/currency_tables.php.\n\n";
echo "pub static REGION_CURRENCY: &[(&str, &str)] = &[\n";
foreach (array_keys($regions) as $r) {
  $f = new NumberFormatter("en_$r", NumberFormatter::CURRENCY);
  $c = $f->getTextAttribute(NumberFormatter::CURRENCY_CODE);
  if ($c !== 'XXX' && $c !== false && $c !== '') echo "    (\"$r\", \"$c\"),\n";
}
echo "];\n";
