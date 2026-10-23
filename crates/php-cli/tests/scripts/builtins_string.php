<?php
// String functions, with the edges PHP defines.
echo chr(65), chr(321), chr(-1) === chr(255) ? "wrap" : "no", ord("A"), ord(""), "\n";
echo strtolower("HeLLo ÄÖ"), strtoupper("hello"), ucfirst("hello"), lcfirst("HELLO"), "\n";
echo ucwords("hello world-foo bar_baz"), "|", ucwords("hello world-foo", "-"), "\n";
var_dump(trim("  x  "), trim("xxhixx", "x"), ltrim("0012", "0"), rtrim("abc\n"), trim("abcba", "a..b"), chop("x  "));
var_dump(str_contains("abc", ""), str_contains("abc", "d"), str_starts_with("abc", "ab"), str_ends_with("abc", "bc"));
var_dump(strpos("hello", "l"), strpos("hello", "l", 3), strpos("hello", "l", -2), strpos("hello", "z"), stripos("HeLLo", "ll"));
var_dump(strrpos("hello", "l"), strrpos("hello", "l", -3), strrpos("hello", "l", 4), strripos("HELLO", "l"));
var_dump(substr("hello", 1), substr("hello", -3, 2), substr("hello", 1, -1), substr("hello", 10), substr("hello", 0, -10), substr("hello", -10, 2));
var_dump(substr_count("hello hello", "ll"), substr_count("aaa", "aa"), substr_count("hello", "l", 3), substr_count("hello", "l", -2, 1));
var_dump(str_pad("5", 3, "0", STR_PAD_LEFT), str_pad("x", 6, "ab", STR_PAD_BOTH), str_pad("abc", 2), str_pad("a", 4));
var_dump(str_split("abcde", 2), str_split(""));
var_dump(implode(", ", [1, 2.5, true, null, "x"]), implode([1, 2]), join("-", []));
var_dump(explode(",", "a,b,,c"), explode(",", "a,b,c", 2), explode(",", "a,b,c", -1), explode(",", "a,b,c", 0), explode(",", ""));
var_dump(strrev("abc"), strcmp("a", "b"), strcmp("b", "a"), strcmp("a", "a"), strcasecmp("HELLO", "hello"), strncmp("abcd", "abef", 2), strncasecmp("AB", "abc", 3));
var_dump(str_replace("l", "L", "hello"), str_replace(["a", "b"], ["b", "c"], "ab"), str_replace(["a", "b"], "x", "abc"), str_replace("a", "b", ["a", "ba", 5]), str_ireplace("L", "x", "HelLo"));
str_replace("o", "0", "foo boo", $n);
var_dump($n);
var_dump(strtr("Hi all", "ai", "eo"), strtr("Hi all", ["Hi" => "Hello", "all" => "world", "a" => "A"]), strtr("abc", []));
var_dump(strstr("user@example.com", "@"), strstr("user@example.com", "@", true), stristr("HELLO", "ell"), strrchr("a/b/c", "/"), strstr("abc", "z"));
var_dump(substr_replace("Hello", "J", 0, 1), substr_replace("Hello", "!!", -1), substr_replace("Hello", "X", 2, 0));
var_dump(nl2br("a\nb\r\nc\rd"), nl2br("x\n", false));
var_dump(htmlspecialchars("<a href='x'>T&amp;C \"q\"</a>"), htmlspecialchars("&amp; &", ENT_QUOTES, "UTF-8", false));
var_dump(addslashes("O'Re\"il\\ly\0"), stripslashes("O\\'Re\\\\il\\0"), bin2hex("abc\xff"));
var_dump(basename("/a/b/c.txt"), basename("/a/b/c.txt", ".txt"), basename("/a/b/"), basename("/"), basename("c"), basename(".txt", ".txt"));
var_dump(dirname("/a/b/c.txt"), dirname("/a/b/c", 2), dirname("a"), dirname("/a"), dirname("/"), dirname(""), dirname("a/b//"));
var_dump(vsprintf("%s-%05d-%.2f", ["x", 42, 3.14159]));
vprintf("%s is %d\n", ["age", 30]);
