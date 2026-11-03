<?php
// Locale: ICU's locale IDs (canonical form, parts, keywords), RFC 4647
// matching and lookup, Accept-Language negotiation, display names.
var_dump(extension_loaded('intl'), class_exists('Locale'), defined('INTL_ICU_VERSION'));
ini_set('intl.default_locale', 'de_DE');
var_dump(Locale::getDefault(), locale_get_default());
var_dump(Locale::setDefault('fr_CA'), Locale::getDefault());

foreach (['en_us', 'EN-us', 'de-DE-u-co-phonebk', 'zh-Hant-TW', 'sr_Latn_RS_REVISED@currency=USD', 'eng_US', 'de-DEVA', 'en__POSIX', 'en_US.UTF-8', 'x-klingon', 'i-klingon', 'und_US', 'en_US_', 'es-419-u-cu-usd', 'en-US-u-ca-gregory-co-phonebk', 'DE_de@CURRENCY=eur', 'de-1996-1901'] as $l) {
    echo str_pad($l, 32), ' ', Locale::canonicalize($l), ' | ', Locale::getPrimaryLanguage($l), ' | ', Locale::getScript($l), ' | ', Locale::getRegion($l), ' | ', json_encode(Locale::getAllVariants($l)), ' | ', json_encode(Locale::getKeywords($l)), ' | ', json_encode(Locale::parseLocale($l)), "\n";
}

var_dump(Locale::composeLocale(['language' => 'de', 'script' => 'Latn', 'region' => 'DE', 'variant0' => 'X', 'private' => ['a1', 'b2']]));
var_dump(Locale::composeLocale(['grandfathered' => 'i-klingon']));
try {
    Locale::composeLocale(['region' => 'DE']);
} catch (ValueError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
var_dump(Locale::composeLocale([]));

var_dump(Locale::filterMatches('de-DEVA', 'de-DE', false), Locale::filterMatches('de-DE', 'de', false), Locale::filterMatches('de_DE@x=y', 'de-de', true), Locale::filterMatches('en', '*'));
var_dump(Locale::lookup(['de-DE', 'en-US'], 'en_US_x', true, 'fr'), Locale::lookup(['de-DE', 'en-US'], 'en-US-x-y'), Locale::lookup(['de-DE'], 'fr', false, 'it'), Locale::lookup([], 'fr'));
var_dump(Locale::acceptFromHttp('da, en-gb;q=0.8, en;q=0.7'), Locale::acceptFromHttp('en-US,en;q=0.9'), Locale::acceptFromHttp('fr-CA;q=0.5, de'), Locale::acceptFromHttp('xx'), Locale::acceptFromHttp('zh-TW'));

foreach (['de_DE', 'en_US', 'fr', 'zh_Hant_TW', 'sr_Latn_RS', 'pt_BR', 'es_419', 'ja'] as $l) {
    echo $l, ': ', Locale::getDisplayName($l, 'en'), ' / ', Locale::getDisplayName($l, 'de'), ' / ', Locale::getDisplayLanguage($l, 'fr'), ' / ', Locale::getDisplayRegion($l, 'en'), "\n";
}
var_dump(Locale::getDisplayScript('sr_Latn_RS', 'en'), Locale::getDisplayScript('ru_Cyrl', 'de'), Locale::getDisplayVariant('en_US_POSIX', 'en'), Locale::getDisplayLanguage('i-klingon', 'en'), Locale::getDisplayRegion('i-klingon', 'en'));

var_dump(Locale::getKeywords('en'), Locale::getKeywords('en@foo=bar!'), intl_get_error_code(), intl_get_error_message());
var_dump(Locale::canonicalize(str_repeat('a', 200)), intl_get_error_message());
var_dump(intl_error_name(U_ILLEGAL_ARGUMENT_ERROR), intl_error_name(U_ZERO_ERROR), intl_error_name(9999), intl_is_failure(U_USING_FALLBACK_WARNING), intl_is_failure(U_ILLEGAL_ARGUMENT_ERROR));
