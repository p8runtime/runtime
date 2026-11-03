<?php
// Named arguments skip optional builtin parameters; the skipped ones take
// their defaults, string escapes included ("\"", "\\", "\n", "\t").
var_dump(str_getcsv("a,\"b c\"", escape: "\\"));
var_dump(str_pad("x", 3, pad_type: STR_PAD_LEFT));
var_dump(explode(separator: ",", string: "1,2"));
var_dump(htmlspecialchars("<a href='x'>", double_encode: false));
var_dump(json_encode(['a' => 1], depth: 4));
