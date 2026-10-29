<?php
// EUC-JP and BIG-5 in mbstring (WordPress's tests convert with both):
// round trips, what neither can hold becoming ?, malformed input, names.
$u = "aあb漢字カナ";
var_dump(bin2hex(mb_convert_encoding($u, 'EUC-JP', 'UTF-8')), mb_convert_encoding(mb_convert_encoding($u, 'EUC-JP', 'UTF-8'), 'UTF-8', 'EUC-JP'));
var_dump(bin2hex(mb_convert_encoding("中文測試", 'BIG-5', 'UTF-8')), mb_convert_encoding("\xa4\xa4\xa4\xe5", 'UTF-8', 'BIG-5'));
var_dump(bin2hex(mb_convert_encoding("\u{1F600}x", 'EUC-JP', 'UTF-8')), bin2hex(mb_convert_encoding("\u{1F600}x", 'BIG-5', 'UTF-8')));
var_dump(mb_convert_encoding("\xff\xfe", 'UTF-8', 'EUC-JP'), mb_check_encoding("\xa4\xa2", 'EUC-JP'), mb_check_encoding("\xa4", 'EUC-JP'), mb_strlen("\xa4\xa2\xa4\xa4", 'EUC-JP'));
var_dump(mb_encoding_aliases('BIG-5'), mb_encoding_aliases('EUC-JP'), mb_preferred_mime_name('BIG-5'), mb_preferred_mime_name('eucJP'));
