<?php
// mbstring's encodings: what invalid input and unmappable characters become.
error_reporting(E_ALL & ~E_DEPRECATED);
$bad = "a\xc3(b\xe2\x82c\xf0\x9f\x98d\xed\xa0\x80e\xc0\xaff";
var_dump(bin2hex(mb_convert_encoding($bad, "UTF-8", "UTF-8")), mb_check_encoding($bad, "UTF-8"), mb_strlen($bad), mb_scrub($bad));
foreach (["UTF-16", "UTF-16BE", "UTF-16LE", "UTF-32", "UTF-32LE", "UCS-2", "UCS-2LE", "UCS-4", "ASCII", "ISO-8859-2", "ISO-8859-5", "ISO-8859-7", "Windows-1251", "Windows-1252", "Windows-1254", "KOI8-R", "CP850", "CP866", "UTF-7", "UTF7-IMAP", "HTML-ENTITIES", "BASE64", "Quoted-Printable", "UUENCODE", "7bit", "8bit"] as $e) {
    $enc = mb_convert_encoding("Aé€日😀Ж", $e, "UTF-8");
    $back = mb_convert_encoding($enc, "UTF-8", $e);
    echo str_pad($e, 17), bin2hex($enc), " | ", bin2hex($back), "\n";
}
foreach (["UTF-16BE" => "\xd8\x3d\xde\x00\xde\x00\x00", "UTF-16LE" => "\x3d\xd8", "UTF-32BE" => "\x00\x11\x00\x00", "UCS-2BE" => "\x00", "ASCII" => "\x80", "Windows-1252" => "\x81\x80", "ISO-8859-3" => "\xa5"] as $e => $in) {
    echo str_pad($e, 14), bin2hex(mb_convert_encoding($in, "UTF-8", $e)), "\n";
}
var_dump(mb_convert_encoding("abc def", "BASE64"), mb_convert_encoding("YWJjIGRlZg==", "8bit", "BASE64"), mb_convert_encoding("caf\xe9 =", "Quoted-Printable", "ISO-8859-1"), mb_convert_encoding("caf=E9", "ISO-8859-1", "Quoted-Printable") === "caf\xe9");
var_dump(mb_convert_encoding("<p>é&amp;€</p>", "HTML-ENTITIES", "UTF-8"), mb_convert_encoding("&eacute;&#233;&#xe9;&bogus;&", "UTF-8", "HTML-ENTITIES"));
var_dump(mb_strtoupper("ǆ ß ﬁ ŉ ΐ"), mb_strtolower("İ Σ"), mb_convert_case("ǆ ǅ ß", MB_CASE_TITLE), mb_convert_case("ǆ ß ﬁ", MB_CASE_TITLE_SIMPLE), mb_convert_case("Σ İ", MB_CASE_FOLD), mb_strtoupper("i", "ISO-8859-9"), mb_strtolower("I", "ISO-8859-9"));
var_dump(mb_strwidth("ｱｲｳ日本語🙂"), mb_strimwidth("日本語", 0, 5, "…"), mb_strimwidth("abcdef", -3, 3), mb_strimwidth("abcdef", 2, 10, "..."));
var_dump(mb_str_split("日本語", 2, "UTF-8"), mb_substr("日本語", 1, null, "UTF-8"), mb_substr("abc", 1, null, "UTF-16LE") === "", bin2hex(mb_substr("a\x00b\x00c\x00", 1, 1, "UTF-16LE")));
var_dump(mb_detect_encoding("\xc3\xa9", "UTF-8, ISO-8859-1"), mb_detect_encoding("\xe9", "UTF-8, ISO-8859-1", true), mb_detect_encoding("\xe9", ["UTF-8"], true), mb_detect_encoding("plain", "auto"));
var_dump(mb_ord("\xff"), mb_ord("\xc3"), mb_chr(0xd800), mb_chr(0x110000), mb_chr(0x41, "UTF-16BE") === "\x00A");
mb_internal_encoding("ISO-8859-1");
var_dump(mb_strlen("\xe9\xe8"), mb_strtoupper("\xe9"), bin2hex(mb_strtoupper("\xe9")), mb_internal_encoding());
mb_internal_encoding("UTF-8");
var_dump(mb_str_pad("é", 3, "日"), mb_check_encoding(["a", ["é", "\xff"]], "UTF-8"), mb_check_encoding(["a", ["é"]], "UTF-8"));
var_dump(mb_convert_variables("UTF-8", "ISO-8859-1", $a1, $a2) ?? null);
$x = "\xe9";
$y = ["k" => "\xe9"];
var_dump(mb_convert_variables("UTF-8", "ISO-8859-1", $x, $y), $x, $y);
mb_parse_str("a=1&b[]=2&b[]=%C3%A9", $out);
var_dump($out);
var_dump(bin2hex(mb_substr($bad, 1, 6)), bin2hex(mb_substr($bad, -5)), mb_strpos($bad, "e"), mb_strlen($bad), count(mb_str_split($bad)), mb_strwidth($bad), bin2hex(mb_strcut("\xe6\x97\xa5\xe6\x9c\xac", 1, 3)), mb_stripos("ÀÉ", "é"), mb_strrpos("日本日本", "日", -2), mb_strrpos("日本日本", "本", -3));
