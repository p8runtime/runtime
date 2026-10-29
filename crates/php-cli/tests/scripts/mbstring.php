<?php
// mbstring over UTF-8, what PHPUnit, Laravel, Symfony and WordPress call.
var_dump(extension_loaded('mbstring'));
var_dump(mb_internal_encoding());
$s = "Zażółć gęślą jaźń — 日本語テキスト 😀";
var_dump(strlen($s), mb_strlen($s));
var_dump(mb_substr($s, 3, 5), mb_substr($s, -4), mb_substr($s, 2, -3), mb_substr($s, 100));
var_dump(mb_strpos($s, 'ń'), mb_strrpos($s, 'ę'), mb_stripos($s, 'GĘŚL'), mb_strripos($s, 'Ż'), mb_strpos($s, 'x'));
var_dump(mb_strtoupper($s), mb_strtolower("ÀÉÎÕÜ ΣΑΣ"), mb_convert_case("hello wORLD o'neil élan", MB_CASE_TITLE));
var_dump(mb_convert_case("Straße", MB_CASE_UPPER), mb_convert_case("Straße", MB_CASE_UPPER_SIMPLE), mb_convert_case("ǆ", MB_CASE_TITLE), mb_convert_case("ß", MB_CASE_FOLD));
var_dump(mb_str_split("añb日😀", 2), mb_str_split(""));
var_dump(mb_strwidth("日本語abc"), mb_strimwidth("日本語テキスト", 0, 9, "..."), mb_strimwidth("Hello World", 0, 20, "..."));
var_dump(mb_str_pad("日本", 6, "ab", STR_PAD_BOTH), mb_str_pad("x", 4, "é", STR_PAD_LEFT));
var_dump(mb_trim("  \u{3000}日本\u{a0} "), mb_ltrim("xxabcxx", "x"), mb_rtrim("xxabcxx", "x"));
var_dump(mb_ucfirst("élan vital"), mb_lcfirst("Élan"));
var_dump(mb_substr_count("ababab", "ab"), mb_substr_count("日本日本日", "日"));
var_dump(mb_strstr("user@example.com", "@"), mb_strstr("user@example.com", "@", true), mb_strrchr("a/b/c", "/"), mb_stristr("HeLLo", "ll"), mb_strrichr("aXbxc", "X", true));
var_dump(mb_ord("😀"), mb_chr(0x1F600), mb_chr(233), mb_ord("é", "ISO-8859-1"));
var_dump(mb_strcut("日本語", 1, 4), mb_strcut("日本語", 3, 3));
var_dump(mb_check_encoding("abc\xff", "UTF-8"), mb_check_encoding("日本", "UTF-8"), mb_scrub("a\xffb"));
var_dump(mb_detect_encoding("日本語", ["ASCII", "UTF-8"], true), mb_detect_encoding("abc"), mb_detect_encoding("\xe9t\xe9", ["UTF-8", "ISO-8859-1"], true));
var_dump(bin2hex(mb_convert_encoding("é€", "ISO-8859-15", "UTF-8")), mb_convert_encoding("\xe9", "UTF-8", "ISO-8859-1"), bin2hex(mb_convert_encoding("é€😀", "UTF-16BE", "UTF-8")), bin2hex(mb_convert_encoding("é", "Windows-1252")));
var_dump(mb_convert_encoding("日", "ASCII", "UTF-8"), mb_convert_encoding("&amp;&#26085;", "UTF-8", "HTML-ENTITIES"));
mb_substitute_character(0x3f);
var_dump(mb_substitute_character(), mb_convert_encoding("日a", "ISO-8859-1", "UTF-8"));
mb_substitute_character("long");
var_dump(mb_convert_encoding("日a", "ISO-8859-1", "UTF-8"));
mb_substitute_character("entity");
var_dump(mb_convert_encoding("日a", "ISO-8859-1", "UTF-8"));
mb_substitute_character("none");
var_dump(mb_convert_encoding("日a", "ISO-8859-1", "UTF-8"));
mb_substitute_character(63);
var_dump(mb_list_encodings() === array_values(array_unique(mb_list_encodings())), in_array("UTF-8", mb_list_encodings()), mb_encoding_aliases("ASCII"));
var_dump(mb_preferred_mime_name("utf8"), mb_preferred_mime_name("ISO-8859-1"));
foreach ([fn() => mb_strlen("a", "nope"), fn() => mb_substr_count("a", ""), fn() => mb_str_split("a", 0), fn() => mb_convert_case("a", 99), fn() => mb_strpos("abc", "b", 10), fn() => mb_substitute_character("x")] as $f) {
    try {
        $f();
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
}
var_dump(mb_encode_numericentity("aé日", [0x80, 0x10ffff, 0, 0xffffff], "UTF-8"), mb_encode_numericentity("é", [0x80, 0xff, 0, 0xff], "UTF-8", true), mb_decode_numericentity("a&#233;&#x65E5;&#128512;", [0x80, 0x10ffff, 0, 0xffffff], "UTF-8"));
var_dump(mb_encode_mimeheader("Prüfung 日本語", "UTF-8", "B"), mb_encode_mimeheader("Prüfung", "UTF-8", "Q"), mb_decode_mimeheader("=?UTF-8?B?UHLDvGZ1bmc=?= x"), mb_decode_mimeheader("=?ISO-8859-1?Q?Pr=FCfung?="));
var_dump(mb_http_output(), mb_language(), mb_get_info("internal_encoding"), mb_detect_order());
var_dump(mb_strlen("\xff\xfe", "UTF-8"), mb_strlen("abcd", "UTF-16LE"), mb_strlen("ab", "8bit"));
var_dump(bin2hex(mb_convert_encoding("aé", "UTF-7")), mb_convert_encoding("+AOk-", "UTF-8", "UTF-7"));
var_dump(mb_convert_encoding(["k" => "é", "n" => [1, "é"]], "ISO-8859-1", "UTF-8") === ["k" => "\xe9", "n" => [1, "\xe9"]]);
