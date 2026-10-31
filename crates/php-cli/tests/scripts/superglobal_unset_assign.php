<?php
function set1() { $_SESSION = ['a' => 1]; }
function get1() { return $_SESSION['a'] ?? 'none'; }
set1(); var_dump(get1(), $_SESSION['a'] ?? 'none-top');
unset($GLOBALS['_SESSION']);
var_dump(isset($_SESSION));
set1(); var_dump(get1(), $_SESSION['a'] ?? 'none-top');
function un() { unset($GLOBALS['_GET']); $_GET = ['g' => 2]; }
un(); var_dump($_GET);
function un2() { unset($_POST); $_POST = ['p' => 3]; } un2(); var_dump($_POST);
